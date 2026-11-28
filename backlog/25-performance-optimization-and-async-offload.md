# 25. Оптимизация производительности: разгрузка UI/async-потоков и рендеринга

**Status:** [x] DONE

**Приоритет:** P1 (Высокий)  
**Платформы:** Windows, Linux, Android

## Контекст
- По результатам статического аудита Rust-ядра, Tauri-бэкенда и React/CSS-фронтенда выявлен ряд узких мест, вызывающих блокировку Tokio-воркеров / главного IPC-потока, лишние JNI/SQLite-вызовы и просадки FPS при скролле (особенно в Android WebView).

## Задачи
- [x] 1. **[P0] Вынести запись и ротацию `RotatingLogSink` из синхронного `Mutex` в фоновый поток**:
   - `src-tauri/src/logging.rs` (`write_bytes`): заменить открытие файла на каждый лог и синхронный `fs::read` + `fs::write` под глобальным `Mutex` на `BufWriter<File>` / фоновый поток через `mpsc`-канал.
- [x] 2. **[P0] Перевести тяжёлые синхронные `#[tauri::command]` в `async fn` + `spawn_blocking`**:
   - `src-tauri/src/commands.rs`: `list_peers`, `list_friend_requests`, `list_history`, `shares_list_local`, `shares_add_local`, `list_lan_interfaces`, `get_qr_connect_info`, `clipboard_push_local_image_bytes`.
   - Также обернуть синхронный обход директорий `collect_outgoing_items` (`crates/landa-core/src/transfer.rs`) в `tokio::task::spawn_blocking` при вызове из `send_files`.
- [x] 3. **[P0] Оптимизировать `discovery.rs` (кэш сокетов, интерфейсы, SQLite)**:
   - `crates/landa-core/src/discovery.rs` (`send_beacon`, `handle_datagram`): кэшировать per-interface UDP-сокеты (пересоздавать только при изменении интерфейсов), убрать дублирующие вызовы `enumerate_*` (и лишние JNI `getLanInterfaces` на Android), вызывать `upsert_peer` только при реальном изменении данных пира или через `spawn_blocking`.
- [x] 4. **[P1] Убрать массовый `backdrop-filter: blur(...)` с мелких повторяющихся элементов**:
   - `src/styles/glass.css`, `src/components/glass/GlassPrimitives.tsx`: убрать `backdrop-filter` с кнопок внутри строк списков (`GlassButton variant="glass"`), оставив полупрозрачный фон + border, и сохранить `backdrop-filter` только для верхнеуровневых контейнеров (`GlassPillNav`, `GlassModal`, основные карточки).
- [x] 5. **[P1] Оптимизировать `SharesView` (общий `IntersectionObserver`, виртуализация, миниатюры)**:
   - `src/features/shares/SharesView.tsx`: заменить отдельный `IntersectionObserver` на каждую строку на один общий наблюдатель (и добавить виртуализацию при >100 элементах), отдавать миниатюры через `PreviewServer` / `Blob` URL вместо base64 `dataUrl`.
- [x] 6. **[P1] Оптимизировать `authorize_preview_target` и дебаунс `checkSharedAndScan`**:
   - `src-tauri/src/preview.rs` (`authorize_preview_target`): сначала проверять `download_dir` / `cache_dir` / `shares`, а к `list_history` обращаться только при промахе (через `spawn_blocking` / точечный SQL-запрос).
   - `src/App.tsx` (`checkSharedAndScan`): добавить debounce/cooldown (300–500 мс) на `window.focus` и `document.visibilitychange`, чтобы избежать двойного вызова `triggerScan()` и IPC.
