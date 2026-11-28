# 26. Максимальная оптимизация RAM и веса приложения перед финальным релизом

**Status:** [x] DONE

**Приоритет:** P1 (Высокий)  
**Платформы:** Windows, Linux, Android

## Контекст
- Перед финальным релизом проведён полный аудит потребления оперативной памяти (Rust-ядро, SQLite, Tokio, Android-сервис, WebView/React) и веса артефактов (`landa-app.exe` — 10.59 МБ, `liblanda_app_lib.so` arm64 — 11.89 МБ, `app-arm64-release.apk` — 20.29 МБ, `app-x86_64-release.apk` — 38.47 МБ).
- Выявлены крупные резервы по снижению пикового и фонового потребления RAM (до сотен МБ при декодировании изображений и до ~100 МБ в кэшах миниатюр) и уменьшению веса бинарников/APK (на ~6–8 МБ на Android и ~15–25% на десктопе).

## Задачи

### Часть A — Оптимизация RAM (Rust, Android, WebView)

- [x] 1. **[P0] Ограничить пиковую RAM при декодировании изображений и кэшировании миниатюр (Rust)**:
   - `crates/landa-mod-shares/src/lib.rs`:
     - Снизить `THUMB_CACHE_MAX_BYTES` (сейчас 2 отдельных кэша `local_thumb_cache` и `remote_thumb_cache` по 50 МиБ = до **100 МиБ** в RAM) до 8–16 МиБ (или общего бюджета на Android) и хранить `Arc<[u8]>` вместо `Vec<u8>`, чтобы не клонировать байты при каждом попадании в кэш.
     - Задать `image::Limits` (`max_image_width`, `max_image_height`, `max_alloc`) в `generate_image_thumbnail` (сейчас фото 12000×9000 аллоцирует ~432 МиБ RGBA в RAM).
     - Отдавать миниатюры через локальный HTTP `PreviewServer` вместо base64 `dataUrl` через Tauri IPC (~33% оверхеда).
   - `crates/landa-mod-clipboard/src/lib.rs`:
     - Снизить `MAX_IMAGE_DIMENSION` (сейчас `8192` → до **256 МиБ** `RgbaImage` в `normalize_to_png`) до `4096` или задать `image::Limits::max_alloc`.
     - Потоково читать/писать изображения буфера обмена через временные файлы вместо буферизации `Vec<u8>` до 10 МиБ целиком в RAM (`tokio::fs::read`, `receive_bytes_with_digest`, `ensure_remote_clipboard_image_cached` в `src-tauri/src/commands.rs`).

- [x] 2. **[P0] Устранить утечки и лишние аллокации RAM во фронтенде и WebView**:
   - `src/features/clipboard/clipboardPaste.ts` и `src/core/api.ts`: заменить `Array.from(new Uint8Array(buf))` (создаёт JS-массив до 10 млн чисел + JSON-сериализацию при вставке картинки до 10 МБ) на передачу бинарного payload через Tauri v2 raw IPC (`Uint8Array` / `tauri::ipc::Request`).
   - `src/features/clipboard/ClipboardView.tsx`:
     - Конвертировать `data:` URL в `Blob` URL с `URL.revokeObjectURL` при вытеснении из `imagePreviewCache` (сейчас хранятся сырые base64-строки, а в `onClick`-ветке отсутствует проверка лимита 100 записей).
     - Заменить отдельный `new IntersectionObserver` в каждом `ClipboardImageThumb` на один общий наблюдатель (по аналогии с `SharesView.tsx`).
   - `src/features/shares/SharesView.tsx`: снизить лимит `thumbMemoryCache` (сейчас 300 `blob:` URL живут бессрочно на уровне модуля) до 100–150 и очищать/подрезать кэш при смене пира или уходе со вкладки; снизить `VIRTUALIZE_THRESHOLD` со 100 до 40–50 для Android WebView.
   - `src/App.tsx`, `src/features/activity/ActivityView.tsx`, `src/features/devices/DevicesView.tsx`:
     - Добавить флаг `disposed` в `bootstrap()` (`App.tsx`) для гарантированной отписки от 4 `listen(...)` при раннем размонтировании; сохранять и очищать `setTimeout` в `showToast` и `DevicesView.tsx`.
     - Загружать `history` (до 100 записей с массивами путей) лениво при открытии `ActivityView` и очищать `resolvedNames` при сбросе/обновлении истории.

- [x] 3. **[P1] Оптимизировать память Android-сервиса, WebView, Tokio, QUIC и SQLite**:
   - `src-tauri/gen/android/app/src/main/java/dev/landa/app/LandaAndroidPlugin.kt`:
     - Обнулять `webViewRef = null` в `onDestroy` (сейчас возможна утечка `WebView`/`Activity`).
     - Ограничить размер/время жизни `ConcurrentHashMap` (`publishedUris`, `publishedDisplayPaths`, `lastProgressNotifMs`) и заменить ad-hoc `Thread { ... }` на ограниченный `ExecutorService`.
     - Оценить вынос `TransferForegroundService` в отдельный процесс или выгрузку/очистку WebView-кэшей при уходе UI в фон (чтобы фоновый сервис не держал ~30+ МБ Chromium WebView в RAM, когда экран закрыт).
   - `crates/landa-core/src/storage.rs`: явно задать `PRAGMA cache_size = -1024;` (1 МиБ) и `PRAGMA journal_size_limit = 4194304;` (4 МиБ) для ограничения роста WAL и page cache.
   - `crates/landa-core/src/transport.rs`: настроить `quinn::TransportConfig` (`stream_receive_window`, `receive_window` до 512 КиБ–2 МиБ на Android) вместо дефолтных окон Quinn.
   - `src-tauri/src/lib.rs`: на Android рассмотреть настройку компактного Tokio runtime (2–4 worker-потока вместо `num_cpus` с 2 МиБ стека на поток).
   - `src/styles/glass.css`: убрать постоянный `will-change: transform, opacity` с `.animate-glass-view` и `.animate-glass-modal` (оставить только на время анимации) и проверить снижение радиуса `backdrop-filter` на вложенных карточках для экономии GPU/compositor-памяти WebView.

### Часть B — Оптимизация веса бинарников, APK и фронтенд-бандла

- [x] 4. **[P1] Оптимизировать Rust-зависимости и release-профиль (`Cargo.toml`)**:
   - `Cargo.toml` и `src-tauri/Cargo.toml` (оба workspace): протестировать `lto = "fat"` (вместо `"thin"`) и `opt-level = "z"` (вместо `"s"`), замерить размер `landa-app.exe` и `liblanda_app_lib.so`.
   - `src-tauri/Cargo.toml`:
     - Убрать дубликат `webpki-roots = "0.26"` (сейчас `reqwest 0.13` тянет `webpki-roots 1.0.12`, в бинарник попадают две таблицы корневых сертификатов Mozilla — ~250+ КБ).
     - Убрать лишние фичи `reqwest`: `charset` (тянет `encoding_rs` с таблицами кодировок) и при возможности `http2` (тянет `h2`) — экономия ~500–800 КБ.
     - Заменить `tracing-subscriber` фичу `env-filter` (тянет `regex` + `regex-syntax`, ~500–800 КБ) на `LevelFilter` / простой парсер `RUST_LOG` в `src-tauri/src/logging.rs`.
   - `crates/landa-mod-shares/Cargo.toml`, `crates/landa-mod-clipboard/Cargo.toml`, `src-tauri/Cargo.toml`: проверить необходимость всех кодеков `image` (`png`, `jpeg`, `webp`, `gif`) и отключить неиспользуемые.

- [x] 5. **[P1] Сократить вес Android APK (MLKit, ProGuard/R8, ABI, ресурсы)**:
   - `src-tauri/gen/android/app/build.gradle.kts`: оценить замену `com.google.mlkit:barcode-scanning:17.3.0` (добавляет **4.72 МБ** `libbarhopper_v3.so` + **860 КБ** `.tflite` моделей в каждый APK) на `com.google.android.gms:play-services-mlkit-barcode-scanning` (тонкий клиент, экономия ~5.5 МБ; учесть риск для устройств без Google Play Services) или лёгкий QR-декодер.
   - `src-tauri/gen/android/app/proguard-rules.pro`: сузить широкие правила `-keep class androidx.camera.** { *; }`, `-keep class com.google.mlkit.** { *; }`, `-keep class app.tauri.** { *; }` до реальных JNI/reflection точек входа, чтобы R8 мог вырезать неиспользуемый код из `classes.dex` (сейчас 4.02 МБ).
   - Проверить, почему `lib/x86_64/liblanda_app_lib.so` весит **29.43 МБ** против **11.88 МБ** у `arm64-v8a` (возможен не-stripped или устаревший артефакт).
   - Сжать/конвертировать в WebP fallback-иконки `mipmap-*` (~315 КБ, используются только на Android < 8).

- [x] 6. **[P2] Code splitting фронтенда и очистка мелкого мусора**:
   - `src/App.tsx`: перевести нестартовые вкладки (`SharesView`, `ClipboardView`, `VideoLinkView`, `ActivityView`, `SettingsView`) и `FilePreviewModal` на `React.lazy()` + `<Suspense>`, чтобы уменьшить начальный JS-чанк и не парсить `VideoLinkView` на Android.
   - `vite.config.ts`: задать явный современный `build.target` (например, `es2021` / `chrome105` / `safari15`) и при необходимости `terser` для финального релиза.
   - `public/icon.svg`: минифицировать через SVGO (убрать комментарии и лишние атрибуты).
   - Удалить локальный мусорный лог `src-tauri/target/desktop_clipboard_retry.log` и `epic025_task2_verify.log` перед релизной сборкой.
