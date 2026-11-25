# Backlog — Landa 2

Список задач и фич, которые ещё предстоит реализовать или перенести из оригинальной Landa в новую модульную архитектуру на Rust + Tauri v2.

## К выполнению

| # | Файл | Задача | Приоритет |
|---|---|---|---|
| 17 | [17-clipboard-sync-and-ui-fix.md](./17-clipboard-sync-and-ui-fix.md) | Починка модуля буфера обмена (`Clipboard`) и бага отправки текста целевому устройству | P0 (Критический) |
| 18 | [18-android-to-pc-shares-folder-download.md](./18-android-to-pc-shares-folder-download.md) | Починка скачивания папок из `Shares` с Android на ПК | P1 (Высокий) |
| 19 | [19-downloads-storage-and-app-size-accounting.md](./19-downloads-storage-and-app-size-accounting.md) | Исключение скачанных файлов и кэшей из «веса приложения» на Android | P1 (Высокий) |
| 20 | [20-svg-app-icon-and-android-launcher.md](./20-svg-app-icon-and-android-launcher.md) | Переделка PNG-иконки в SVG и замена иконки приложения везде (включая Android) | P1 (Высокий) |
| 21 | [21-android-apk-size-reduction.md](./21-android-apk-size-reduction.md) | Уменьшение веса Android APK | P2 (Средний) |

## Done

| # | Файл | Задача | Status |
|---|---|---|---|
| 16 | [16-android-connection-and-shares-recovery.md](./16-android-connection-and-shares-recovery.md) | Стабильность подключения на Android и восстановление видимости устройств и `Shares` | Done |
| 15 | [15-android-automatic-update.md](./15-android-automatic-update.md) | Автоматический запуск установки обновлений на Android | Done |
| 14 | [14-shares-folder-download-and-paths.md](./14-shares-folder-download-and-paths.md) | Скачивание папок целиком в `Shares` и отображение путей к файлам | Done |
| 13 | [13-fullscreen-gesture-preview-and-ux.md](./13-fullscreen-gesture-preview-and-ux.md) | Починка предпросмотра файлов, полноэкранный режим с жестами и удаление кнопки «глазика» | Done |
| 12 | [12-ui-layout-modals-and-devices.md](./12-ui-layout-modals-and-devices.md) | Исправление шапки, модальных окон и управление устройствами (забыть устройство) | Done |
| 11 | [11-android-play-protect-fix.md](./11-android-play-protect-fix.md) | Устранение предупреждения Google Play Protect на Android APK | Done |
| 10 | [10-diagnostics-and-release-signing.md](./10-diagnostics-and-release-signing.md) | Просмотр логов, выбор сетевых интерфейсов и релизная подпись Android/CI | Done |
| 01 | [01-auto-update.md](./01-auto-update.md) | Кнопка автообновления (стрелочка слева от «Обновить») и проверка релизов GitHub | Done |
| 02 | [02-android-folder-send.md](./02-android-folder-send.md) | Отправка папок на Android (SAF `ACTION_OPEN_DOCUMENT_TREE`) | Done |
| 03 | [03-in-app-file-preview.md](./03-in-app-file-preview.md) | Лёгкий встроенный предпросмотр фото, видео, аудио и текста (без `libmpv`) | Done |
| 04 | [04-shares-lazy-thumbnails.md](./04-shares-lazy-thumbnails.md) | Ленивые превью изображений/видео в модуле `Shares` по QUIC | Done |
| 05 | [05-desktop-drag-and-drop.md](./05-desktop-drag-and-drop.md) | Drag & Drop файлов прямо на карточку устройства на десктопе | Done |
| 06 | [06-android-background-and-alerts.md](./06-android-background-and-alerts.md) | Foreground Service на Android и уведомления о входящих запросах в фоне | Done |
| 07 | [07-clipboard-images.md](./07-clipboard-images.md) | Поддержка изображений (PNG) в модуле `Clipboard` | Done |
| 08 | [08-qr-quick-connect.md](./08-qr-quick-connect.md) | Быстрое подключение и спаривание по QR-коду поверх QUIC | Done |
| 09 | [09-video-link-module.md](./09-video-link-module.md) | Опциональный модуль веб-ссылки на видео (`landa-mod-videolink`) | Done |
