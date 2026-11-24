# 12. Исправление шапки, модальных окон и управление устройствами

**Status:** [x] DONE

**Приоритет:** P0 (Критический)  
**Платформы:** Android, Windows, Linux

## Контекст (подтверждено через ADB на Xiaomi `25102PCBEG`)
- На стартовом экране «Устройства» заголовок `Устройства` + бейдж имени устройства (`POCO F8 Ultra`) не помещаются в одну строку с 4 кнопками действий (QR-сканер, QR-код, обновление, сканирование), из-за чего бейдж наезжает на кнопки.
- Модальные окна на главном экране (`GlassModal` в `DevicesView.tsx`: обновление, QR-код, переименование, drag&drop) рендерятся внутри `<div key={activeTab} className="animate-glass-view">` (`src/App.tsx:400`), где CSS-анимация `glassViewEnter` (`src/styles/glass.css:135-171`) оставляет `transform` и `filter` с `animation-fill-mode: both`. Это создаёт новый containing block для `position: fixed`, из-за чего модалка обрезается областью вкладки, смещается вверх и наезжает на интерфейс, а полупрозрачный фон `--glass-bg-elevated` (`rgba(255,255,255,0.12)`) пропускает текст подложки.
- В `DevicesView` и `landa-core` есть только `unfriend` (сброс флага доверия), но нет возможности полностью забыть/удалить устройство из списка и базы.

## Задачи
- [x] 1. **[P0] Починить модальные окна на главном экране и во всех вкладках**:
   - Перенести рендер `GlassModal` (`src/components/glass/GlassPrimitives.tsx`) в React Portal (`createPortal(..., document.body)`), чтобы `position: fixed; inset: 0` всегда привязывался к viewport, а не к контейнеру вкладки.
   - Убрать остаточные `transform`/`filter` после завершения `glassViewEnter` в `src/styles/glass.css` (или не использовать `both`/`forwards` на свойствах, создающих containing block).
   - Усилить контраст и непрозрачность подложки/карточки модального окна (`bg-black/65` + более плотный фон карточки в тёмной и светлой темах), чтобы текст интерфейса под модалкой не просвечивал и не смешивался с содержимым модалки.
   - Проверить через `adb` на устройстве все модалки на главном экране (Обновление, QR-код, Переименование, Входящий запрос). *(Проверено на Xiaomi `25102PCBEG` через `adb` на актуальной сборке `0.6.1`: Обновление — `target/adb-modal-v2-update.png`, QR-код — `target/adb-modal-v2-qr.png`, Переименование — `target/adb-modal-v2-rename.png`, реальный входящий запрос передачи от ПК `Landa-Test-PC` — `target/adb-modal-v2-incoming-offer.png`, реальный входящий запрос в друзья — `target/adb-modal-v2-friend-request.png`. Все карточки центрированы по viewport, не обрезаются, текст и кнопки читаемы без просвечивания подложки.)*
- [x] 2. **[P0] Убрать надпись «Устройства» сверху слева и освободить место в шапке**:
   - Удалить `<h1 ...>{t.devices.title}</h1>` из шапки `src/features/devices/DevicesView.tsx`.
   - Оставить слева компактный бейдж локального устройства (`identity.deviceName` с `min-w-0 truncate`), а справа — кнопки действий с достаточным зазором, чтобы на узких мобильных экранах (~360–400dp) элементы не наезжали друг на друга.
   - Проверить вёрстку шапки через `adb` (включая режим для левши `leftHandedMode`). *(Проверено на Xiaomi `25102PCBEG` через `adb` при ширине `400dp` (`1200x2608 @ 480dpi`) с длинным именем `POCO F8 Ultra_Very_Long_Device_Name_Test`: в обычном режиме — `target/adb-header-long-normal.png`, в режиме для левши `leftHandedMode` — `target/adb-header-long-lefthanded.png`, скрипт измерения и исходный stdout сохранены в `target/epic-012-task2-verification.log`. Имя усекается с многоточием, все 4 кнопки полностью видны, между бейджем и кнопками сохраняется зазор `36 px` / `12 dp` (`gap-3`).)*
- [x] 3. **[P1] Добавить возможность забывать устройства**:
   - В `crates/landa-core/src/storage.rs` и `crates/landa-core/src/engine.rs` добавить метод `forget_peer(peer_id)` (удаление записи из таблиц `peers` и `friend_requests`, очистка из кэша discovery и эмиссия `CoreEvent::PeersChanged`).
   - В `src-tauri/src/commands.rs` и `src/core/api.ts` добавить команду `forget_peer`.
   - В `src/features/devices/DevicesView.tsx` добавить действие «Забыть устройство» (с подтверждением или понятной иконкой/меню на карточке устройства) и переводы в `src/i18n/translations.ts` (RU/EN).
   - Покрыть unit-тестом в `landa-core` удаление пира и связанных запросов дружбы. *(Проверено в текущем рабочем дереве: `cargo test -p landa-core forget_peer` — 2/2 passed (`storage::tests::storage_forget_peer_removes_peer_and_friend_requests`, `engine::tests::engine_forget_peer_clears_storage_discovery_cache_and_emits_event`), `npm test` — 12/12 passed, `npm run build` — exit 0; исходный вывод команд сохранён в `target/epic-012-task3-verification.log`.)*
