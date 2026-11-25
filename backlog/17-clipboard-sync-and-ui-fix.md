# 17. Починка модуля буфера обмена (Clipboard) и бага отправки текста

**Status:** [-] IN PROGRESS

**Приоритет:** P0 (Критический)  
**Платформы:** Android, Windows, Linux

## Контекст
- Модуль буфера обмена (`Clipboard`) не работает как ожидается.
- В интерфейсе (`src/features/clipboard/ClipboardView.tsx`) есть баг: при выборе целевого устройства текст якобы можно отправить (показывается уведомление «Отправлено»), но на целевом устройстве и в списке ничего не происходит.
- По анализу кода выявлены причины:
  - В `ClipboardView.tsx:172-199` при `targetId !== "local"` вызываются `clipboardSendToPeer` и `clipboardPushLocalText`, но не обновляется `remoteItems` (`loadRemote(targetId)`), а `useEffect` на `trustedOnlinePeers` пересоздаётся при каждом обновлении списка пиров и может сбрасывать `targetId` на `"local"`.
  - В `crates/landa-mod-clipboard/src/lib.rs:531-570` входящий `ClipboardRequest::PushText` / `PushImage` только сохраняется во внутреннюю историю `ClipboardModule` и шлёт `ModuleEvent`, но не записывается в системный буфер обмена получателя и не сопровождается уведомлением/индикацией.
  - На Android (`LandaAndroidPlugin.kt:773-922`) чтение/запись системного буфера (`ClipboardManager`) подвержены ограничениям Android 10+ (доступ только при фокусе окна) и требуют проверки работы с текстом и `FileProvider` URI изображений; на Desktop (`src-tauri/src/commands.rs`) `arboard::Clipboard` может падать при блокировке буфера другим процессом или на Wayland.

## Задачи
- [-] 1. **[P0] Исправить UI и поток отправки текста/изображений целевому устройству**:
   - В `src/features/clipboard/ClipboardView.tsx`: после успешной отправки текста или изображения целевому устройству обновлять список (`loadRemote(targetId)`), блокировать повторную отправку на время запроса (`sending`), корректно отображать ошибки доставки и стабилизировать `useEffect` (не сбрасывать выбранного пира при фоновом обновлении `peers`).
   - В `crates/landa-mod-clipboard/src/lib.rs` и `src-tauri/src/lib.rs`: при получении текста/изображения от доверенного пира автоматически копировать входящий текст в системный буфер обмена (с опцией в настройках или безопасным fallback, если приложение в фоне на Android) и показывать уведомление/тост о получении.
- [ ] 2. **[P0] Починить чтение и запись системного буфера обмена на Android и Desktop**:
   - В `LandaAndroidPlugin.kt`: проверить и исправить `readSystemClipboard`, `writeSystemClipboardText` и `writeSystemClipboardImage` (учёт фокуса окна на Android 10+, корректная обработка `ClipData` и `FLAG_GRANT_READ_URI_PERMISSION` для изображений).
   - В `src-tauri/src/commands.rs`: сделать работу с `arboard::Clipboard` устойчивой к временным ошибкам/пересозданию инстанса на Windows и Linux.
   - Добавить unit/интеграционные тесты на отправку/получение текста и изображений в `crates/landa-mod-clipboard` и UI-тест `ClipboardView`.
