# Landa 2 (Rust + Tauri v2 + Apple Liquid Glass)

Чистая модульная переработка Landa с нуля на **Rust + Tauri v2 + React/Tailwind CSS** под **Windows, Linux и Android**.

## Архитектура

- `crates/landa-proto` — протокол v2:
  - Ed25519-идентичность устройства (`PeerId = blake3(pubkey)[..16]`)
  - Подписанные UDP discovery-маяки (`LND2`, порт `40405`)
  - Взаимная TLS 1.3 / QUIC аутентификация по Ed25519-сертификатам
  - Жёсткая валидация путей (`validate_relative_path`, `resolve_within_root`) против CWE-22
  - Ограниченные фреймы (`MAX_CONTROL_FRAME_BYTES = 2 MiB`) против CWE-400
- `crates/landa-core` — ядро:
  - LAN Discovery с фильтрацией VPN/туннельных интерфейсов + ручные IP-цели
  - Единый QUIC-транспорт (`quinn` + `rustls`) и потоковая передача файлов с проверкой **BLAKE3**
  - SQLite (`rusqlite` bundled) для пиров, запросов дружбы, истории и настроек
  - Контракт модульности `LandaModule` + `ModuleRegistry`
- `crates/landa-mod-shares` — опциональный модуль общих папок (ленивый просмотр дерева и скачивание по QUIC без фонового `re-cache`)
- `crates/landa-mod-clipboard` — опциональный модуль буфера обмена (только между доверенными друзьями по QUIC)
- `src-tauri` — Tauri v2 host (трей, single-instance, диалоги, уведомления, системное открытие файлов)
- `src` — React + TypeScript + Tailwind CSS интерфейс в стиле **Apple Liquid Glass** (RU/EN, тёмная/светлая тема, режим для левши)

## Проверка и запуск

```powershell
# 1. Тесты Rust-ядра и модулей
cargo test --workspace

# 2. Установка зависимостей и сборка фронтенда
npm install
npm run build

# 3. Запуск на десктопе (Windows / Linux)
cargo tauri dev

# 4. Инициализация и сборка под Android
cargo tauri android init
cargo tauri android build
```
