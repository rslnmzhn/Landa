# 15. Автоматический запуск установки обновлений на Android

**Status:** [ ] TODO

**Приоритет:** P1 (Высокий)  
**Платформы:** Android

## Контекст
- В рамках EPIC-011 разрешение `REQUEST_INSTALL_PACKAGES` и прямой запуск установщика APK были временно убраны: сейчас Android-клиент скачивает и проверяет APK по SHA-256, сохраняет его в `Downloads/Landa/` и открывает системные «Загрузки», где пользователь должен вручную найти и запустить файл.
- Пользователю нужен полностью автоматический сценарий обновления на Android: нажал «Обновить» → APK скачался, проверился по SHA-256 → сразу открылся системный установщик обновления приложения.

## Задачи
- [ ] 1. **[P1] Восстановить автоматический запуск установки APK после скачивания и проверки SHA-256**:
   - Вернуть `REQUEST_INSTALL_PACKAGES` и `<queries>` для `application/vnd.android.package-archive` в `src-tauri/gen/android/app/src/main/AndroidManifest.xml`.
   - В `LandaAndroidPlugin.kt` и `src-tauri/src/updater.rs` / `android_bridge.rs` восстановить автоматический запуск системного установщика (`FileProvider` + `Intent.ACTION_VIEW` с `application/vnd.android.package-archive` и проверкой `canRequestPackageInstalls()` с понятным переходом в настройки разрешения при первом обновлении).
   - В `src/features/devices/DevicesView.tsx` вернуть единый поток «Обновить» → скачивание → проверка SHA-256 → автоматический запуск установки (без ручного шага «Открыть Загрузки»).
   - Обновить проверку разрешений в `.github/workflows/build.yml` (оставить запрет на `MANAGE_EXTERNAL_STORAGE` и `READ_EXTERNAL_STORAGE`, разрешив `REQUEST_INSTALL_PACKAGES`).
- [ ] 2. **[P1] Проверить отсутствие предупреждения Google Play Protect с `REQUEST_INSTALL_PACKAGES`**:
   - Собрать релизный `arm64` APK с постоянной релизной подписью, R8 и без `MANAGE_EXTERNAL_STORAGE`.
   - Проверить установку и сценарий автообновления через `adb` на физическом устройстве (Xiaomi `25102PCBEG` с включённым Play Protect) и убедиться, что предупреждение Play Protect не появляется.
