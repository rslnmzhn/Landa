use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use landa_core::LandaModule;
use landa_mod_videolink::{VideoLinkModule, VideoSource, MAX_HTTP_HEADER_BYTES};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use uuid::Uuid;

async fn send_raw_http(addr: SocketAddr, req: &str) -> (String, Vec<u8>) {
    let mut stream = TcpStream::connect(addr).await.unwrap();
    stream.write_all(req.as_bytes()).await.unwrap();
    stream.flush().await.unwrap();

    let mut raw = Vec::new();
    stream.read_to_end(&mut raw).await.unwrap();

    let split = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .expect("valid HTTP response headers");
    let headers = String::from_utf8_lossy(&raw[..split]).to_string();
    let body = raw[split + 4..].to_vec();
    (headers, body)
}

fn extract_set_cookie(headers: &str) -> Option<String> {
    for line in headers.lines() {
        if let Some((k, v)) = line.split_once(':') {
            if k.trim().eq_ignore_ascii_case("set-cookie") {
                let cookie_pair = v.split(';').next()?.trim();
                return Some(cookie_pair.to_string());
            }
        }
    }
    None
}

#[tokio::test]
async fn videolink_auth_range_streaming_and_stop() {
    let temp_dir = std::env::temp_dir().join(format!("landa-vl-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let video_path = temp_dir.join("sample&clip'1.mp4");
    let payload: Vec<u8> = (0..256u16).map(|i| (i % 251) as u8).collect();
    std::fs::write(&video_path, &payload).unwrap();

    let module = VideoLinkModule::new();
    let status = module
        .start_with_bind(
            Ipv4Addr::LOCALHOST,
            0,
            VideoSource::File(video_path.clone()),
            "secret-pass-42".into(),
        )
        .await
        .unwrap();
    assert!(status.running);
    assert_eq!(status.file_name.as_deref(), Some("sample&clip'1.mp4"));
    assert_eq!(status.size_bytes, Some(payload.len() as u64));

    let addr = module.bound_addr().await.unwrap();

    // 1. Unauthenticated /stream must be rejected with 401 and leak no bytes
    let (hdrs, body) = send_raw_http(
        addr,
        "GET /stream HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
    )
    .await;
    assert!(hdrs.starts_with("HTTP/1.1 401 Unauthorized"), "{hdrs}");
    assert_eq!(body, b"Unauthorized");

    // 2. Unauthenticated /watch renders login form (escaped filename, no <video>)
    let (hdrs, body) = send_raw_http(
        addr,
        "GET /watch HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
    )
    .await;
    assert!(hdrs.starts_with("HTTP/1.1 200 OK"), "{hdrs}");
    let html = String::from_utf8_lossy(&body);
    assert!(html.contains("action=\"/login\""));
    assert!(html.contains("sample&amp;clip&#39;1.mp4"));
    assert!(!html.contains("<video"));

    // 3. Wrong password returns 401 and sets no cookie
    let wrong_body = "password=wrong";
    let (hdrs, _) = send_raw_http(
        addr,
        &format!(
            "POST /login HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/x-www-form-urlencoded\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{wrong_body}",
            wrong_body.len()
        ),
    )
    .await;
    assert!(hdrs.starts_with("HTTP/1.1 401 Unauthorized"), "{hdrs}");
    assert!(extract_set_cookie(&hdrs).is_none());

    // 4. Valid password issues session cookie
    let ok_body = "password=secret-pass-42";
    let (hdrs, _) = send_raw_http(
        addr,
        &format!(
            "POST /login HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/x-www-form-urlencoded\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{ok_body}",
            ok_body.len()
        ),
    )
    .await;
    assert!(hdrs.starts_with("HTTP/1.1 303 See Other"), "{hdrs}");
    let cookie = extract_set_cookie(&hdrs).expect("session cookie must be set");
    assert!(cookie.starts_with("landa_vl_session="));

    // 5. Authenticated /watch renders HTML5 player
    let (hdrs, body) = send_raw_http(
        addr,
        &format!(
            "GET /watch HTTP/1.1\r\nHost: localhost\r\nCookie: {cookie}\r\nConnection: close\r\n\r\n"
        ),
    )
    .await;
    assert!(hdrs.starts_with("HTTP/1.1 200 OK"), "{hdrs}");
    let player_html = String::from_utf8_lossy(&body);
    assert!(player_html.contains("<video"));
    assert!(player_html.contains("src=\"/stream\""));

    // 6. Authenticated /stream with Range returns 206 Partial Content and exact slice
    let (hdrs, body) = send_raw_http(
        addr,
        &format!(
            "GET /stream HTTP/1.1\r\nHost: localhost\r\nCookie: {cookie}\r\nRange: bytes=10-24\r\nConnection: close\r\n\r\n"
        ),
    )
    .await;
    assert!(hdrs.starts_with("HTTP/1.1 206 Partial Content"), "{hdrs}");
    let hdrs_lower = hdrs.to_ascii_lowercase();
    assert!(
        hdrs_lower.contains("content-range: bytes 10-24/256"),
        "{hdrs}"
    );
    assert!(hdrs_lower.contains("accept-ranges: bytes"), "{hdrs}");
    assert_eq!(body, &payload[10..=24]);

    // 7. Out-of-bounds Range returns 416
    let (hdrs, _) = send_raw_http(
        addr,
        &format!(
            "GET /stream HTTP/1.1\r\nHost: localhost\r\nCookie: {cookie}\r\nRange: bytes=9999-\r\nConnection: close\r\n\r\n"
        ),
    )
    .await;
    assert!(
        hdrs.starts_with("HTTP/1.1 416 Range Not Satisfiable"),
        "{hdrs}"
    );

    // 8. Stopping server closes listener immediately
    let stopped = module.stop().await;
    assert!(!stopped.running);
    assert!(!module.status().running);
    assert!(TcpStream::connect(addr).await.is_err());

    // 9. Disabling module blocks starting
    module.set_enabled(false);
    let disabled_res = module
        .start_with_bind(
            Ipv4Addr::LOCALHOST,
            0,
            VideoSource::File(video_path),
            "secret-pass-42".into(),
        )
        .await;
    assert!(disabled_res.is_err());

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[tokio::test]
async fn videolink_concurrent_starts_never_orphan_listeners() {
    let temp_dir = std::env::temp_dir().join(format!("landa-vl-conc-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let video_path = temp_dir.join("video.mp4");
    std::fs::write(&video_path, b"0123456789abcdef").unwrap();

    let module = VideoLinkModule::new();
    let mut handles = Vec::new();
    for i in 0..6 {
        let m = Arc::clone(&module);
        let p = video_path.clone();
        handles.push(tokio::spawn(async move {
            m.start_with_bind(
                Ipv4Addr::LOCALHOST,
                0,
                VideoSource::File(p),
                format!("pass-{i}"),
            )
            .await
        }));
    }

    let mut issued_ports = Vec::new();
    for h in handles {
        let status = h.await.unwrap().unwrap();
        assert!(status.running);
        if let Some(port) = status.port {
            issued_ports.push(port);
        }
    }
    assert_eq!(issued_ports.len(), 6);

    let stopped = module.stop().await;
    assert!(!stopped.running);
    assert!(!module.status().running);

    for port in issued_ports {
        let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
        assert!(
            TcpStream::connect(addr).await.is_err(),
            "port {port} remained open after stop()"
        );
    }

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[tokio::test]
async fn videolink_disable_during_start_leaves_no_open_port() {
    let temp_dir = std::env::temp_dir().join(format!("landa-vl-dis-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let video_path = temp_dir.join("video.mp4");
    std::fs::write(&video_path, b"0123456789abcdef").unwrap();

    for _ in 0..10 {
        let module = VideoLinkModule::new();
        let m_start = Arc::clone(&module);
        let p = video_path.clone();
        let start_task = tokio::spawn(async move {
            m_start
                .start_with_bind(
                    Ipv4Addr::LOCALHOST,
                    0,
                    VideoSource::File(p),
                    "pass-123".into(),
                )
                .await
        });

        tokio::task::yield_now().await;
        module.set_enabled(false);

        let res = start_task.await.unwrap();
        // Give the cancelled server task a moment to drop its listener if it started right before set_enabled(false).
        tokio::time::sleep(Duration::from_millis(20)).await;

        assert!(!module.status().running);
        if let Ok(status) = res {
            if let Some(port) = status.port {
                let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
                assert!(
                    TcpStream::connect(addr).await.is_err(),
                    "port {port} accepted connections after set_enabled(false)"
                );
            }
        }
        let _ = module.stop().await;
    }

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[tokio::test]
async fn videolink_rejects_oversized_header_without_waiting_for_newline() {
    let temp_dir = std::env::temp_dir().join(format!("landa-vl-hdr-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let video_path = temp_dir.join("video.mp4");
    std::fs::write(&video_path, b"0123456789abcdef").unwrap();

    let module = VideoLinkModule::new();
    module
        .start_with_bind(
            Ipv4Addr::LOCALHOST,
            0,
            VideoSource::File(video_path),
            "secret".into(),
        )
        .await
        .unwrap();
    let addr = module.bound_addr().await.unwrap();

    // Case 1: Oversized request line with NO newline and write half kept open
    {
        let mut stream = TcpStream::connect(addr).await.unwrap();
        let mut payload = Vec::with_capacity(MAX_HTTP_HEADER_BYTES + 1024);
        payload.extend_from_slice(b"GET /");
        payload.resize(MAX_HTTP_HEADER_BYTES + 1024, b'A');
        stream.write_all(&payload).await.unwrap();
        stream.flush().await.unwrap();

        let mut resp = Vec::new();
        tokio::time::timeout(Duration::from_secs(2), stream.read_to_end(&mut resp))
            .await
            .expect("server must respond 431 immediately without waiting for newline")
            .unwrap();
        let resp_str = String::from_utf8_lossy(&resp);
        assert!(
            resp_str.starts_with("HTTP/1.1 431 Request Header Fields Too Large"),
            "{resp_str}"
        );
    }

    // Case 2: Valid request line followed by oversized header line with NO newline and write half kept open
    {
        let mut stream = TcpStream::connect(addr).await.unwrap();
        let mut payload = Vec::with_capacity(MAX_HTTP_HEADER_BYTES + 1024);
        payload.extend_from_slice(b"GET /watch HTTP/1.1\r\nX-Huge: ");
        payload.resize(MAX_HTTP_HEADER_BYTES + 1024, b'B');
        stream.write_all(&payload).await.unwrap();
        stream.flush().await.unwrap();

        let mut resp = Vec::new();
        tokio::time::timeout(Duration::from_secs(2), stream.read_to_end(&mut resp))
            .await
            .expect("server must respond 431 immediately without waiting for newline")
            .unwrap();
        let resp_str = String::from_utf8_lossy(&resp);
        assert!(
            resp_str.starts_with("HTTP/1.1 431 Request Header Fields Too Large"),
            "{resp_str}"
        );
    }

    module.stop().await;
    let _ = std::fs::remove_dir_all(&temp_dir);
}
