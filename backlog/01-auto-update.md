# 01. Автообновление приложения и кнопка в шапке

**Статус:** Выполнено  
**Приоритет:** Высокий  
**Платформы:** Windows, Linux, Android

## Что нужно сделать
1. **UI в `DevicesView.tsx`**:
   - Добавить компактную стеклянную кнопку со стрелочкой (например, `ArrowDownCircle` / `Download` или `ArrowUpRight`) **слева от кнопки «Обновить»** в шапке экрана устройств.
   - Показывать индикатор (точку/бейдж), если найдена новая версия в GitHub Releases.
   - По нажатию открывать компактную стеклянную модалку с версией, списком изменений, прогрессом скачивания и кнопкой «Установить».
2. **Rust-сервис проверки и загрузки (`crates/landa-core` или `src-tauri`)**:
   - Запрос к `https://api.github.com/repos/rslnmzhn/Landa/releases/latest`.
   - Поиск ассета `landa-vX.Y.Z-release-manifest.json` (который уже генерируется в CI/CD).
   - Сравнение текущей версии (`env!("CARGO_PKG_VERSION")`) с `release.version`.
   - Скачивание подходящего под платформу и архитектуру файла (`windows-x64-setup.exe`, `linux-x86_64.AppImage`, `android-arm64-v8a.apk` и т.д.) с обязательной проверкой **SHA-256** из манифеста и защитой имени файла от Path Traversal.
3. **Установка по платформам**:
   - **Windows**: запуск скачанного `setup.exe` и выход из приложения.
   - **Linux**: `chmod +x` для `.AppImage` и открытие папки / перезапуск.
   - **Android**: добавить `REQUEST_INSTALL_PACKAGES` в `AndroidManifest.xml` и метод `installApkUpdate` в `LandaAndroidPlugin.kt` через `FileProvider` + `Intent.ACTION_VIEW` (`application/vnd.android.package-archive`).
