# 19. Исключение скачанных файлов и кэшей из веса приложения

**Status:** [x] DONE

**Приоритет:** P1 (Высокий)  
**Платформы:** Android, Windows, Linux

## Контекст
- Сейчас при скачивании файлов внутри приложения на Android они учитываются системой как вес самого приложения («Данные приложения» / «Кэш»), что создаёт ошибочное впечатление о размере приложения и раздувает занимаемое место.
- По анализу кода выявлены конкретные причины:
  - В `crates/landa-mod-shares/src/lib.rs:507-551` метод `download_remote_file` (скачивание одиночного файла из `Shares`) **не использует** `engine.receive_publish_hook()` (`MediaStore.Downloads`), а пишет файл напрямую в `engine.get_settings().download_dir`, который на Android указывает на приватную директорию приложения (`/data/user/0/dev.landa.app/...`), а не в публичную папку `Downloads/Landa`.
  - В `src-tauri/src/updater.rs:498-505` скачанные APK обновлений (~40 МБ) сохраняются в `app_cache_dir/updates` и никогда не удаляются после установки.
  - В `src-tauri/src/preview.rs:147-179` предпросмотр удалённых файлов из `Shares` (до 100 МБ на файл) кэшируется в `app_cache_dir/preview_cache` с очисткой только раз в сутки при старте; также не очищаются осиротевшие файлы в `incoming_temp` после прерванных передач.

## Задачи
- [x] 1. **[P1] Публиковать одиночные файлы из `Shares` в системный `MediaStore.Downloads`**:
   - В `crates/landa-mod-shares/src/lib.rs` (`download_remote_file`): скачивать файл во временный `incoming_temp`, проверять BLAKE3 и публиковать через `engine.receive_publish_hook()` (на Android — в `MediaStore.Downloads` → `Downloads/Landa/...`) с немедленным удалением временного файла, аналогично `download_remote_folder` и `TransferManager`.
   - Проверить, что на Android в `SettingsView` и истории передач отображается публичный путь `Downloads/Landa/...`, а файл не остаётся в приватной папке приложения.
- [x] 2. **[P1] Очистка и квотирование внутренних кэшей приложения**:
   - В `src-tauri/src/updater.rs`: удалять устаревшие/установленные APK из `app_cache_dir/updates` при запуске приложения (если текущая версия >= версии APK в кэше) и после запуска установки.
   - В `src-tauri/src/preview.rs`, `crates/landa-core/src/transfer.rs` и `LandaAndroidPlugin.kt`: очищать `incoming_temp` при старте, ввести жёсткий LRU-лимит размера для `preview_cache` и `share_intent`, а также добавить в `SettingsView.tsx` отображение размера кэша и кнопку «Очистить кэш».
