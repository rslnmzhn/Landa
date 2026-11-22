# Backlog — Landa 2

Список задач и фич, которые ещё предстоит реализовать или перенести из оригинальной Landa в новую модульную архитектуру на Rust + Tauri v2.

## К выполнению

| # | Файл | Задача | Приоритет |
|---|---|---|---|
| 11 | [11-android-play-protect-fix.md](./11-android-play-protect-fix.md) | Устранение предупреждения Google Play Protect на Android APK | Высокий |
| 10 | [10-diagnostics-and-release-signing.md](./10-diagnostics-and-release-signing.md) | Просмотр логов, выбор сетевых интерфейсов и релизная подпись Android/CI | Средний |
| 08 | [08-qr-quick-connect.md](./08-qr-quick-connect.md) | Быстрое подключение и спаривание по QR-коду поверх QUIC | Низкий |
| 09 | [09-video-link-module.md](./09-video-link-module.md) | Опциональный модуль веб-ссылки на видео (`landa-mod-videolink`) | Низкий |

## Done

| # | Файл | Задача | Status |
|---|---|---|---|
| 01 | [01-auto-update.md](./01-auto-update.md) | Кнопка автообновления (стрелочка слева от «Обновить») и проверка релизов GitHub | Done |
| 02 | [02-android-folder-send.md](./02-android-folder-send.md) | Отправка папок на Android (SAF `ACTION_OPEN_DOCUMENT_TREE`) | Done |
| 03 | [03-in-app-file-preview.md](./03-in-app-file-preview.md) | Лёгкий встроенный предпросмотр фото, видео, аудио и текста (без `libmpv`) | Done |
| 04 | [04-shares-lazy-thumbnails.md](./04-shares-lazy-thumbnails.md) | Ленивые превью изображений/видео в модуле `Shares` по QUIC | Done |
| 05 | [05-desktop-drag-and-drop.md](./05-desktop-drag-and-drop.md) | Drag & Drop файлов прямо на карточку устройства на десктопе | Done |
| 06 | [06-android-background-and-alerts.md](./06-android-background-and-alerts.md) | Foreground Service на Android и уведомления о входящих запросах в фоне | Done |
| 07 | [07-clipboard-images.md](./07-clipboard-images.md) | Поддержка изображений (PNG) в модуле `Clipboard` | Done |
