# 18. Починка скачивания папок из Shares с Android на ПК

**Status:** [ ] TODO

**Приоритет:** P1 (Высокий)  
**Платформы:** Android, Windows, Linux

## Контекст
- Скачивание папки целиком в `Shares` с ПК на Android работает корректно, но в обратном направлении — с Android на ПК — не работает.
- По анализу кода выявлены вероятные точки отказа на стороне Android SAF-провайдера и протокола `Shares`:
  - В `crates/landa-mod-shares/src/lib.rs` (`validate_tree_entries`, `build_folder_manifest`) все пути из `ShareContentProvider::list_tree` проверяются через строгий `validate_relative_path` (`crates/landa-proto/src/path_policy.rs`), который отклоняет всю папку целиком, если на Android в имени файла/подпапки встречается символ `:` (частый случай для скриншотов/медиафайлов), хвостовой пробел/точка или длина сегмента > 120 символов.
  - В `LandaAndroidPlugin.kt` (`listSafShareTree`, `openSafShareFile`, `resolveSafTreeDocId`) обход SAF-дерева выполняется синхронно на binder-потоке, а размер файла в манифесте берётся из `COLUMN_SIZE` (который у некоторых SAF-провайдеров может быть `0`, `-1` или отличаться от фактического размера потока при `openFileDescriptor`), из-за чего `stream_exact_file_with_blake3` завершается с `UnexpectedEof` и сбрасывает QUIC-поток (`send.reset`).

## Задачи
- [ ] 1. **[P1] Исправить обход SAF-дерева и потоковую отдачу файлов с Android**:
   - В `LandaAndroidPlugin.kt`: перенести `listSafShareTree` в фоновый поток (или оптимизировать запросы `ContentResolver`), корректно обрабатывать файлы с отсутствующим/нулевым `COLUMN_SIZE` и проверять реальный размер при `openSafShareFile`.
   - В `crates/landa-mod-shares/src/lib.rs`: при формировании манифеста папки на стороне отправителя санитаризовать имена сегментов для целевой ОС (сохраняя исходный `source_rel` для `open_file`), чтобы файлы с `:` или хвостовыми точками/пробелами на Android не ломали скачивание на Windows; при несовпадении размера файла отдавать понятную ошибку до сброса потока.
- [ ] 2. **[P1] Тестирование и проверка сценария Android → ПК**:
   - В `crates/landa-mod-shares/tests/shares_test.rs`: добавить интеграционный тест с mock `ShareContentProvider`, имитирующим Android SAF (вложенные папки, файлы с `:` в имени, пустые подпапки, проверка BLAKE3).
   - Проверить реальное скачивание папки с Android (Xiaomi `25102PCBEG`) на Windows/Linux через `SharesView`.
