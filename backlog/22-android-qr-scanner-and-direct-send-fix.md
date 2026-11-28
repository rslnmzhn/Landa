# 22. Починка краша QR-сканера и прямой отправки файлов с Android на ПК

**Status:** [x] DONE

**Приоритет:** P0 (Критический)  
**Платформы:** Android (отправка, QR-сканер), Desktop (приём)

## Контекст
- После сборки и установки релизного Android APK (`app-arm64-release.apk`) выявлены два критических регрессионных/функциональных бага на устройстве:
  1. **Краш при открытии QR-сканера**: при попытке открыть `QrScannerActivity` приложение аварийно завершается (вероятные причины: R8/ProGuard-минификация классов CameraX/MLKit/Activity, отсутствие keep-правил, тема/ресурсы `AppCompatActivity` или `resourceConfigurations` / `packaging.resources.excludes`).
  2. **Не работает прямая отправка файлов с Android на ПК**: при выборе файла и отправке напрямую с Android на ПК возникает ошибка, что файл не найден (вероятно, передача `content://` URI вместо доступного пути или ошибка чтения/резолва SAF URI в `LandaAndroidPlugin.kt` / `android_bridge.rs` / `commands.rs` / `transfer.rs`).

## Задачи
- [x] 1. **[P0] Диагностика и починка краша при открытии QR-сканера на Android**:
   - Снять стектрейс краша через `adb logcat` при открытии QR-сканера в релизной сборке (`app-arm64-release.apk`).
   - Проверить `QrScannerActivity.kt`, `AndroidManifest.xml`, `proguard-rules.pro` (R8 keep-правила для `QrScannerActivity`, CameraX, MLKit `barcode-scanning`) и настройки `build.gradle.kts` (`isMinifyEnabled`, `isShrinkResources`, `packaging.resources.excludes`).
   - Исправить причину краша и проверить открытие сканера, запрос разрешения камеры и успешное сканирование QR-кода на физическом Android-устройстве.
- [x] 2. **[P0] Диагностика и починка прямой отправки файлов с Android на ПК**:
   - Воспроизвести прямую отправку файла с Android на ПК и снять логи (`adb logcat` + логи приложения) с точным текстом ошибки «не может найти файл».
   - Проверить цепочку выбора и чтения файла на Android (`LandaAndroidPlugin.kt`, `SafShareHelper.kt`, `android_bridge.rs`, `src-tauri/src/commands.rs`, `crates/landa-core/src/transfer.rs`): обработку `content://` URI, временных/кэшированных путей и прав доступа SAF.
   - Исправить резолв/чтение файла при прямой отправке, добавить unit/integration-тест на регрессию и проверить реальную передачу файла с Android на ПК.
