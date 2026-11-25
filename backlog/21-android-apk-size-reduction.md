# 21. Уменьшение веса Android APK

**Status:** [ ] TODO

**Приоритет:** P2 (Средний)  
**Платформы:** Android

## Контекст
- Текущий релизный `arm64` APK (`app-arm64-release.apk`) весит ~40.2 МБ, из которых ~31.7 МБ занимает нативная библиотека `liblanda_app_lib.so` и ~1.5 МБ — несжатый `icon.png`.
- В корневом `Cargo.toml` и `src-tauri/Cargo.toml` отсутствует секция `[profile.release]` (нет `lto`, `opt-level = "z"`/`"s"`, `strip`, `codegen-units = 1`, `panic = "abort"`).
- Задача имеет более низкий приоритет, но желательна для ускорения скачивания и обновлений.

## Задачи
- [ ] 1. **[P2] Оптимизация Rust release-профиля и нативной библиотеки**:
   - Добавить `[profile.release]` в корневой `Cargo.toml` (`opt-level = "z"` или `"s"`, `lto = "thin"`/`true`, `codegen-units = 1`, `strip = "symbols"`, `panic = "abort"`).
   - Замерить размер `liblanda_app_lib.so` и `app-arm64-release.apk` до и после, проверить отсутствие регрессий в скорости QUIC/BLAKE3 и корректность сборки в CI (`.github/workflows/build.yml`).
- [ ] 2. **[P2] Аудит и сокращение ресурсов и зависимостей Android APK**:
   - Убрать лишний вес встроенных ассетов (после замены `icon.png` на SVG/оптимизированные PNG в EPIC-20) и проверить размер `dist/` фронтенд-бандла.
   - Проверить Android Gradle-конфигурацию (`src-tauri/gen/android/app/build.gradle.kts`): `resourceConfigurations` (оставить `ru`, `en`), исключение лишних `META-INF` и неиспользуемых ресурсов, а также оценить вклад `mlkit:barcode-scanning` / `camera-*` и Rust-зависимостей (`image`, `reqwest`, `rusqlite`).
   - Зафиксировать итоговое уменьшение веса APK и проверить установку и работу всех функций на устройстве.
