# EPIC-010: Диагностика, выбор сетевых интерфейсов и релизная подпись

## Контекст и цели
- Дать пользователю контроль над тем, какие локальные LAN-подсети/адаптеры используются для discovery, QR-подключения и VideoLink.
- Обеспечить запись ротируемого лог-файла `landa.log` в `app_data_dir` и быстрый доступ к логам и диагностическому отчёту из `SettingsView`.
- Зафиксировать решения по релизной подписи Android и идентификатору приложения.

## Не-цели
- Не менять `identifier` (`dev.landa.app`) в `src-tauri/tauri.conf.json`, чтобы не ломать обновление существующих Android-установок.
- Не перезаписывать уже настроенные GitHub Secrets (`ANDROID_KEYSTORE_BASE64`, `ANDROID_KEYSTORE_PASSWORD`, `ANDROID_KEY_ALIAS`, `ANDROID_KEY_PASSWORD`).
- Не добавлять внешние крейты ротации логов (`tracing-appender` и т.п.).

## Рассмотренные подходы
- **Вариант A (выбран):** Фильтрация подсетей по CIDR в `AppSettings.excludedSubnets` + встроенный `RotatingLogWriter` для `app_data_dir/landa.log`. Устойчив к переименованию адаптеров, соответствует `DiscoveryNetworkScopeStore`, держит единый файл `landa.log` с последними N строками без лишних зависимостей.
- **Вариант B (отклонён):** Фильтрация по имени интерфейса + `tracing-appender` с дневной ротацией. Ломается при смене имени адаптера и создаёт файлы с датами вместо единого `landa.log`.

## Архитектура решения

### 1. Выбор сетевых интерфейсов и подсетей (`landa-core` + `src-tauri` + `src/`)
- **`crates/landa-core/src/storage.rs`**:
  - В `AppSettings` добавляется `#[serde(default)] pub excluded_subnets: Vec<String>` (JSON: `excludedSubnets`).
- **`crates/landa-core/src/network.rs`**:
  - `LanInterface::subnet_cidr(&self) -> String` вычисляет нормализованный CIDR (`network/prefix`).
  - `parse_ipv4_cidr(raw: &str) -> Option<(Ipv4Addr, u32)>` парсит и нормализует IPv4 CIDR.
  - `set_excluded_subnets(&[String])` / `get_excluded_subnets() -> Vec<String>` управляют глобальным фильтром подсетей (синхронизируется при старте `LandaEngine` и в `update_settings`).
  - `enumerate_all_lan_interfaces()` возвращает все физические LAN-интерфейсы (после фильтра VPN/туннелей).
  - `enumerate_lan_interfaces()` фильтрует `enumerate_all_lan_interfaces()` по `excluded_subnets` — автоматически влияет на `discovery.rs`, `qr.rs`, `transport.rs` и `landa-mod-videolink`.
  - `list_lan_interface_statuses()` возвращает список интерфейсов с `subnet_cidr` и флагом `enabled`.
- **`src-tauri/src/commands.rs`**:
  - Команда `list_lan_interfaces` возвращает `Vec<LanInterfaceInfo>` (`name`, `ip`, `netmask`, `broadcast`, `subnetCidr`, `enabled`).
  - При изменении `excludedSubnets` через `update_settings` вызывается `trigger_scan()`.
- **`src/features/settings/SettingsView.tsx`**:
  - Секция «Сетевые интерфейсы» со списком обнаруженных адаптеров (`name`, `ip`, `subnetCidr`), переключателями включения/исключения подсети и кнопкой обновления списка.

### 2. Ротируемый лог-файл и диагностика (`src-tauri` + `src/`)
- **`src-tauri/src/logging.rs`**:
  - `RotatingLogWriter` реализует `std::io::Write` и `tracing_subscriber::fmt::MakeWriter` поверх `app_data_dir/landa.log`.
  - При превышении лимита (например, `MAX_LOG_LINES = 2000` или `MAX_LOG_BYTES = 512 KiB`) под мьютексом усекает файл до последних `KEEP_LOG_LINES = 1000` строк.
  - `read_log_tail(max_lines)` читает последние строки `landa.log` для диагностического отчёта.
  - `tracing_subscriber` в `src-tauri/src/lib.rs` инициализируется с выводом и в stderr, и в `RotatingLogWriter` (с `with_ansi(false)` для файла).
- **`src-tauri/src/commands.rs`**:
  - `open_logs`: гарантирует существование `landa.log` и открывает его через `preview::open_file_external` (Desktop `opener` / Android `FileProvider`).
  - `copy_diagnostics`: формирует текстовый отчёт (версия приложения, ОС/архитектура, `peerId`, `quicPort`, все LAN-интерфейсы со статусом, исключённые подсети, ручные IP, статусы модулей, последние 200 строк `landa.log`) и копирует в системный буфер обмена (Desktop `arboard` / Android `AndroidBridge::write_system_clipboard_text_async`), возвращая текст отчёта.
- **`src/features/settings/SettingsView.tsx`**:
  - Секция «Диагностика» с кнопками «Открыть логи» и «Скопировать диагностику».

### 3. Релизная подпись Android и `identifier`
- Секреты `ANDROID_KEYSTORE_BASE64`, `ANDROID_KEYSTORE_PASSWORD`, `ANDROID_KEY_ALIAS`, `ANDROID_KEY_PASSWORD` уже присутствуют в GitHub Repository Secrets и подключены в `.github/workflows/build.yml` и `src-tauri/gen/android/app/build.gradle.kts`.
- `identifier` в `src-tauri/tauri.conf.json` остаётся `dev.landa.app`.

## Верификация
- Unit-тесты в `landa-core` на парсинг CIDR, фильтрацию `enumerate_lan_interfaces()` по `excluded_subnets` и сохранение `excludedSubnets` в `AppSettings`.
- Unit-тесты в `src-tauri` на ротацию `RotatingLogWriter` (сохранение последних N строк при превышении порога) и формирование диагностического отчёта.
- `cargo test --workspace`, `cargo clippy --workspace -- -D warnings`, `cargo clippy --manifest-path src-tauri/Cargo.toml -- -D warnings`, `cargo clippy --manifest-path src-tauri/Cargo.toml --target aarch64-linux-android -- -D warnings`, `cargo check --manifest-path src-tauri/Cargo.toml --no-default-features`, `npm test`, `npm run build`.
