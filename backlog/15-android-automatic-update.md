# 15. Автоматический запуск установки обновлений на Android

**Status:** [x] DONE

**Приоритет:** P1 (Высокий)  
**Платформы:** Android

## Контекст
- В рамках EPIC-011 разрешение `REQUEST_INSTALL_PACKAGES` и прямой запуск установщика APK были временно убраны: сейчас Android-клиент скачивает и проверяет APK по SHA-256, сохраняет его в `Downloads/Landa/` и открывает системные «Загрузки», где пользователь должен вручную найти и запустить файл.
- Пользователю нужен полностью автоматический сценарий обновления на Android: нажал «Обновить» → APK скачался, проверился по SHA-256 → сразу открылся системный установщик обновления приложения.

## Задачи
- [x] 1. **[P1] Восстановить автоматический запуск установки APK после скачивания и проверки SHA-256**:
   - Вернуть `REQUEST_INSTALL_PACKAGES` и `<queries>` для `application/vnd.android.package-archive` в `src-tauri/gen/android/app/src/main/AndroidManifest.xml`.
   - В `LandaAndroidPlugin.kt` и `src-tauri/src/updater.rs` / `android_bridge.rs` восстановить автоматический запуск системного установщика (`FileProvider` + `Intent.ACTION_VIEW` с `application/vnd.android.package-archive` и проверкой `canRequestPackageInstalls()` с понятным переходом в настройки разрешения при первом обновлении).
   - В `src/features/devices/DevicesView.tsx` вернуть единый поток «Обновить» → скачивание → проверка SHA-256 → автоматический запуск установки (без ручного шага «Открыть Загрузки»).
   - Обновить проверку разрешений в `.github/workflows/build.yml` (оставить запрет на `MANAGE_EXTERNAL_STORAGE` и `READ_EXTERNAL_STORAGE`, разрешив `REQUEST_INSTALL_PACKAGES`).
- [x] 2. **[P1] Проверить отсутствие предупреждения Google Play Protect с `REQUEST_INSTALL_PACKAGES`**:
   - Собрать релизный `arm64` APK с постоянной релизной подписью, R8 и без `MANAGE_EXTERNAL_STORAGE`.
   - Проверить установку и сценарий автообновления через `adb` на физическом устройстве (Xiaomi `25102PCBEG` с включённым Play Protect) и убедиться, что предупреждение Play Protect не появляется.
   - *(Уточнение критерия согласовано с пользователем: критична угроза/блокировка Play Protect; стандартный запрос первичного сканирования «Рекомендуется проверка приложения» для нового sideloaded APK допустим. Проверено на Xiaomi `25102PCBEG`, Android 16, `package_verifier_state=1`, `package_verifier_user_consent=1`: (1) автообновление из Landa `0.5.0` (`versionCode=5000`) → `0.6.1` (`versionCode=6001`) через `FileProvider` + `com.google.android.packageinstaller` вызвало запрос первичного сканирования «Рекомендуется проверка приложения», после «Проверить» Play Protect выдал «Судя по всему, приложение безопасное — Вы можете установить его» и установил `0.6.1` без блокировки; (2) установка нового `app-arm64-release.apk` (`0.5.0`, `versionCode=5000`, SHA-256 `c07c1003070ac9ff2de61847981fb053c52e621b920325b126e918063ec67d35`, R8, подпись `df7996021e2fd2adf545bd3a28ca36d3b4151a57c2c43a8af49ae6c69335c28e`, с `REQUEST_INSTALL_PACKAGES` и без `MANAGE_EXTERNAL_STORAGE`) через `com.google.android.packageinstaller` прошла без диалогов и предупреждений Play Protect.)*
