# 02. Отправка папок на Android (SAF)

**Status:** Done  
**Приоритет:** Высокий  
**Платформы:** Android

## Проблема
Сейчас на Android кнопка «Папка» в `DevicesView` вызывает `tauri-plugin-dialog` `open({ directory: true })`, который на Android либо не поддерживается, либо возвращает `content://` tree URI, с которым `std::fs::read_dir` в Rust не умеет работать напрямую.

## Что нужно сделать
1. **Kotlin (`LandaAndroidPlugin.kt`)**:
   - Добавить команду `pickFolderForSend` через `Intent(Intent.ACTION_OPEN_DOCUMENT_TREE)` (`startActivityForResult`).
   - Рекурсивно обойти выбранное дерево через `DocumentFile.fromTreeUri` / `DocumentsContract.buildChildDocumentsUriUsingTree`.
   - Либо скопировать выбранные файлы во временный кэш `cacheDir/outgoing_folder/<rootName>/<relPath>` с сохранением структуры подпапок, либо отдавать список `(contentUri, relativePath, size)` для потокового чтения.
   - Очищать временный кэш после завершения или отмены передачи.
2. **Rust (`src-tauri/src/android_bridge.rs`, `commands.rs`)**:
   - Добавить команду `send_folder_android(peer_id)` (или единый `pick_and_send_folder`), которая вызывает Kotlin-пикер и передаёт собранные `OutgoingFileItem` с относительными путями в `TransferManager`.
3. **UI (`DevicesView.tsx`)**:
   - На Android при нажатии на кнопку «Папка» вызывать нативный SAF-пикер вместо `plugin-dialog`.
