# 20. Переделка PNG-иконки в SVG и замена иконки приложения везде (включая Android)

**Status:** [x] DONE

**Приоритет:** P1 (Высокий)  
**Платформы:** Android, Windows, Linux

## Контекст
- Сейчас в `src-tauri/icons/` лежит тяжёлый растровый `icon.png` (~1.5 МБ) и `icon.ico`, а у Android-приложения в `src-tauri/gen/android/app/src/main/res/` до сих пор стоит дефолтная иконка Tauri (`drawable/ic_launcher_background.xml`, `drawable-v24/ic_launcher_foreground.xml`, `mipmap-*/ic_launcher*.png` от `cargo tauri android init`).
- Необходимо перевести текущую PNG-иконку в чистый векторный SVG и заменить иконку приложения на всех платформах, обязательно заменив дефолтную иконку Tauri на Android.

## Задачи
- [x] 1. **[P1] Векторизовать `icon.png` в `icon.svg` и обновить иконки для Desktop/Web**:
   - Создать чистый векторный `src-tauri/icons/icon.svg` на основе текущей иконки `src-tauri/icons/icon.png`.
   - Сгенерировать из `icon.svg` оптимизированные растровые иконки для Tauri (`32x32.png`, `128x128.png`, `128x128@2x.png`, `icon.png`, `icon.ico`) и обновить `src-tauri/tauri.conf.json` и favicon в `index.html`.
- [x] 2. **[P1] Заменить дефолтную иконку Tauri на Android (Adaptive Icon + mipmap)**:
   - Заменить `src-tauri/gen/android/app/src/main/res/drawable/ic_launcher_background.xml` и `drawable-v24/ic_launcher_foreground.xml` на векторные ресурсы Landa (с корректной safe-zone 66/108 dp для адаптивных иконок).
   - Добавить `mipmap-anydpi-v26/ic_launcher.xml` и `mipmap-anydpi-v26/ic_launcher_round.xml`, перегенерировать все `mipmap-{mdpi,hdpi,xhdpi,xxhdpi,xxxhdpi}/ic_launcher*.png` и прописать `android:roundIcon="@mipmap/ic_launcher_round"` в `AndroidManifest.xml`.
   - Проверить отображение новой иконки на физическом Android-устройстве (лаунчер, шторка уведомлений/недавние приложения) и на Windows/Linux (окно, панель задач, системный трей).
