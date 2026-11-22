# 07. Поддержка изображений (PNG) в модуле Clipboard

**Приоритет:** Низкий  
**Платформы:** Windows, Linux, Android

## Что нужно сделать
1. **Rust (`crates/landa-mod-clipboard`)**:
   - Расширить `ClipboardItem` типом контента (`Text` / `Image`), хранить PNG-файлы в `<app_data_dir>/clipboard_images/` с лимитом размера (например, до 10 МБ) и автоочисткой старых записей.
   - Добавить `ClipboardRequest::PushImage` и передачу байтов по QUIC-потоку.
2. **Системный буфер обмена**:
   - На Desktop: чтение/запись изображений из системного буфера (`arboard` или Tauri clipboard plugin).
   - На Android: чтение/запись изображений через `ClipboardManager` + `FileProvider` URI.
3. **UI (`ClipboardView.tsx`)**:
   - Кнопка «Вставить из буфера» (подхватывает и текст, и картинку), превью картинки в карточке и кнопка копирования обратно в буфер.
