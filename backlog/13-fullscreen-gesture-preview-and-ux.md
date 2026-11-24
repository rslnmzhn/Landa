# 13. Починка предпросмотра файлов, полноэкранный просмотр с жестами и очистка UI

**Status:** [x] DONE

**Приоритет:** P0 (Критический)  
**Платформы:** Android, Windows, Linux

## Контекст
- В релизной Android-сборке `AndroidManifest.xml` задаёт `android:usesCleartextTraffic="false"`, при этом отсутствует `network_security_config.xml` с исключением для `127.0.0.1` / `localhost`. Из-за этого WebView блокирует запросы к локальному `PreviewServer` (`http://127.0.0.1:<port>/preview/<token>` в `src-tauri/src/preview.rs`).
- Кроме того, `publishViaMediaStore` в `LandaAndroidPlugin.kt` возвращает путь `MediaStore.MediaColumns.DATA` (`/storage/emulated/0/Download/Landa/...`), который на Android 10+ без `READ_EXTERNAL_STORAGE` не открывается через `std::fs::File::open` в `preview.rs`.
- Текущий `FilePreviewModal.tsx` открывается в ограниченном окне (`max-h-[90vh] max-w-4xl`), содержит кнопки зума (`ZoomIn`, `ZoomOut`, `%`) и не поддерживает мультитач-жесты (pinch-to-zoom) и скрытие/показ оверлея по нажатию на изображение.
- В `SharesView.tsx` и `ActivityView.tsx` рядом с файлами отображается лишняя кнопка «глазика» (`<Eye />`), дублирующая клик по самому файлу.

## Задачи
- [x] 1. **[P0] Починить предпросмотр файлов на Android и десктопе**:
   - Добавить `src-tauri/gen/android/app/src/main/res/xml/network_security_config.xml` с `cleartextTrafficPermitted="true"` строго для `127.0.0.1` и `localhost` (при сохранении запрета cleartext для всех внешних доменов) и подключить его в `AndroidManifest.xml`.
   - В `LandaAndroidPlugin.kt` (`publishViaMediaStore`) и `src-tauri/src/preview.rs` обеспечить корректное открытие принятых файлов на Android Scoped Storage (использовать `content://` URI через `AndroidBridge::open_content_uri_file_async` / `get_content_uri_meta_async` вместо прямого POSIX-доступа к `Download/Landa/...`, если файл недоступен напрямую).
   - Проверить через `adb` на реальном устройстве предпросмотр изображений, видео, аудио и текста из `ActivityView`, `SharesView` и `ClipboardView`. *(Проверено на Xiaomi `25102PCBEG` через `adb` на подписанной release-сборке `0.6.1` (`versionCode=6001`, `785bf65`): `ActivityView` — `target/adb-preview-activity-expanded.png`, `target/adb-preview-act-image.png`, `target/adb-preview-act-video.png`, `target/adb-preview-act-audio.png`, `target/adb-preview-act-text.png`; `SharesView` — `target/adb-preview-shares-image.png`, `target/adb-preview-shares-video.png`, `target/adb-preview-shares-audio.png`, `target/adb-preview-shares-text.png`; `ClipboardView` — `target/adb-preview-clipboard-list.png`, `target/adb-preview-clipboard-modal.png`. Исходный вывод `adb` (модель, версия пакета, фильтрация `logcat` за `17:52..17:58` — 0 ошибок `CLEARTEXT`/`net::ERR_`) сохранён в `target/epic-013-task1-adb-verification.log`, аудит-лог тестов — в `target/epic-013-task1-verification.log`.)*
- [x] 2. **[P1] Полноэкранный просмотрщик с жестами и скрытием интерфейса по тапу (`FilePreviewModal.tsx`)**:
   - Рендерить просмотрщик через React Portal (`document.body`) на весь экран (`fixed inset-0 w-screen h-screen`, без рамок и отступов вокруг медиа).
   - Убрать кнопки приближения/отдаления (`ZoomIn`, `ZoomOut`, кнопку сброса `%`) из панели управления.
   - Реализовать жесты для изображений: pinch-to-zoom двумя пальцами (с плавным масштабированием относительно центра жеста), double-tap для быстрого увеличения/сброса, панорамирование одним пальцем при `scale > 1` (и колесо мыши + drag на десктопе).
   - Реализовать переключение видимости элементов интерфейса (верхняя панель с именем файла, кнопками «Открыть в приложении», «Копировать», «Закрыть») по одиночному нажатию (tap) на изображение — повторное нажатие снова показывает кнопки.
- [x] 3. **[P2] Убрать лишнюю кнопку «глазика» (`<Eye />`)**:
   - Удалить кнопку/иконку `<Eye />` в `src/features/shares/SharesView.tsx` (в режимах «Плитка» и «Список»), оставив открытие предпросмотра по нажатию на карточку/строку файла и кнопку скачивания.
   - Удалить кнопку/иконку `<Eye />` в `src/features/activity/ActivityView.tsx` (для одиночных принятых файлов и в раскрытом списке файлов), оставив открытие предпросмотра по нажатию на элемент.
