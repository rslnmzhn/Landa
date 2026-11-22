# 08. Быстрое подключение и спаривание по QR-коду

**Status:** [x] DONE

**Приоритет:** Низкий  
**Платформы:** Windows, Linux, Android

## Контекст
В оригинальной Landa для этого был отдельный стек `nearby_transfer` на 4 400 строк. В Landa 2 отдельный транспорт не нужен — достаточно передавать в QR-коде параметры уже существующего QUIC-узла.

## Что нужно сделать
- [x] 1. **Формат QR-ссылки**:
   - `landa2://connect?ip=<lan_ip>&port=<quic_port>&peer=<peer_id>&pk=<base64_ed25519_pk>&name=<url_encoded_name>`
- [x] 2. **UI**:
   - Кнопка «Показать QR» в шапке `DevicesView` (генерирует SVG QR-код локального устройства).
   - На Android: кнопка «Сканировать QR» (через CameraX + MLKit Barcode Scanning в `LandaAndroidPlugin.kt`), которая сразу добавляет пира в `DiscoveryService` / `Storage` и открывает прямое QUIC-соединение (полезно в гостевых Wi-Fi сетях с изоляцией broadcast/multicast).
- [x] 3. **Безопасность**:
   - Публичный ключ из QR-кода сразу проверяется при TLS 1.3 / QUIC рукопожатии (`LandaPeerCertVerifier`), а добавление в друзья всё равно подтверждается пользователем.
