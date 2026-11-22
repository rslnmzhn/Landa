# 11. Устранение предупреждения Google Play Protect на Android APK

**Приоритет:** Высокий  
**Платформы:** Android

## Почему Google Play Protect считает APK опасным
1. **Подпись эфемерным debug-ключом в CI**:
   - Сейчас в `src-tauri/gen/android/app/build.gradle.kts` при отсутствии секретов `ANDROID_KEYSTORE_*` релизный APK подписывается `signingConfigs.getByName("debug")`.
   - На каждом запуске GitHub Actions (`ubuntu-latest`) генерируется новый временный `debug.keystore` с `CN=Android Debug`. Play Protect массово помечает sideloaded APK с debug-сертификатом и меняющейся подписью как потенциально опасные.
2. **Комбинация высокорисковых разрешений в `AndroidManifest.xml`**:
   - `MANAGE_EXTERNAL_STORAGE` (доступ ко всем файлам) + `REQUEST_INSTALL_PACKAGES` (установка APK) + `INTERNET` — классический триггер эвристики Play Protect для неизвестных приложений вне Google Play.
3. **Новый `applicationId` (`dev.landa.app`) без репутации и без R8/обфускации/сжатия ресурсов**.

## Что нужно сделать
1. **Постоянный релизный keystore (обязательно)**:
   - Сгенерировать постоянный `.jks` ключ (`keytool -genkeypair ...`) и прописать в GitHub Secrets (`ANDROID_KEYSTORE_BASE64`, `ANDROID_KEYSTORE_PASSWORD`, `ANDROID_KEY_ALIAS`, `ANDROID_KEY_PASSWORD`).
   - В `build.gradle.kts` и CI запретить публикацию release APK с debug-подписью (или явно падать, если секреты не заданы, после их настройки).
2. **Убрать `MANAGE_EXTERNAL_STORAGE` из `AndroidManifest.xml`**:
   - Перевести выбор папок для отправки и модуля `Shares` на Android Storage Access Framework (`ACTION_OPEN_DOCUMENT_TREE` + `takePersistableUriPermission`, см. [02-android-folder-send.md](./02-android-folder-send.md)).
   - Для сохранения принятых файлов уже используется `MediaStore.Downloads`, которому `MANAGE_EXTERNAL_STORAGE` не нужен.
3. **Пересмотреть `REQUEST_INSTALL_PACKAGES` для автообновления**:
   - Проверить, перестанет ли Play Protect ругаться после перехода на постоянный release keystore и удаления `MANAGE_EXTERNAL_STORAGE`.
   - Если `REQUEST_INSTALL_PACKAGES` всё равно будет триггерить Play Protect на чистых устройствах, сделать для Android-сборки вариант обновления через открытие страницы релиза / скачивание APK в системный `Downloads` с подсказкой пользователю (или отдельный flavor с `REQUEST_INSTALL_PACKAGES`).
4. **Hardening релизной сборки Android**:
   - Проверить `isMinifyEnabled = true` и `isShrinkResources = true` в `release` buildType (`proguard-rules.pro` с правилами для `dev.landa.app.*` и `app.tauri.*`).
   - Убедиться, что `usesCleartextTraffic="false"` в release и нет лишних разрешений (`READ_EXTERNAL_STORAGE` / `WRITE_EXTERNAL_STORAGE` только с `maxSdkVersion`).
5. **Верификация / апелляция в Google Play Protect**:
   - После выпуска стабильно подписанного APK без `MANAGE_EXTERNAL_STORAGE` проверить установку на реальном устройстве с включённым Play Protect и при необходимости отправить APK на проверку (Play Protect Appeal / Google Play Console).
