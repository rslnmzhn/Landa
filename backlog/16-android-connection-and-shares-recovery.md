# 16. Стабильность подключения на Android и восстановление видимости устройств и Shares

**Status:** [x] DONE

**Приоритет:** P0 (Критический)  
**Платформы:** Android, Windows, Linux

## Контекст
- При длительном нахождении в приложении или после сворачивания на Android пропадает коннект с устройствами в сети.
- После перезапуска приложения на Android не удаётся ничего скачать: общедоступные папки (`Shares`) с устройств просто не видны.
- По анализу кода выявлены причины:
  - В `LandaAndroidPlugin.kt` `WifiManager.WifiLock` удерживается только внутри `TransferForegroundService` (во время активной передачи), а при простое/в фоне Wi-Fi переходит в энергосбережение и теряет UDP multicast/broadcast пакеты; `MulticastLock` не перезахватывается при смене сети, а `DiscoveryService` (`crates/landa-core/src/discovery.rs`) не пересоздаёт UDP-сокет при сбоях/смене интерфейса.
  - В `crates/landa-core/src/transport.rs` `QuicTransport::bind_ip` не выставляет `SO_REUSEADDR` на `DEFAULT_QUIC_PORT` (`40406`), из-за чего при быстром перезапуске порт может смениться на случайный, а кэшированные `Connection` на удалённых пирах остаются «живыми» локально до истечения `max_idle_timeout` (15 с).
  - На холодном старте Android `LandaEngine::start_loops()` запускается до выдачи runtime-разрешений (`NEARBY_WIFI_DEVICES` / `ACCESS_FINE_LOCATION`), а в `src/features/shares/SharesView.tsx` при кратковременном пропадании пира сбрасывается `selectedPeerId` и нет авто-повтора/кнопки обновления списка шар.

## Задачи
- [x] 1. **[P0] Устойчивость UDP Discovery и QUIC-транспорта на Android при простое, сворачивании и смене сети**:
   - В `LandaAndroidPlugin.kt`: удерживать `WifiManager.WifiLock` (`WIFI_MODE_FULL_LOW_LATENCY` / `WIFI_MODE_FULL_HIGH_PERF`) пока приложение активно на переднем плане (и корректно освобождать в `onPause`/`onDestroy`), перезахватывать `MulticastLock` и уведомлять ядро при `NetworkCallback.onAvailable`/`onLost` и `onResume`.
   - В `crates/landa-core/src/discovery.rs`: при `trigger_scan()` / ошибках `recv_from` / смене сетевых интерфейсов перепривязывать сокет и повторно вступать в multicast-группу `239.255.40.45` на актуальных LAN-интерфейсах.
   - В `crates/landa-core/src/transport.rs`: использовать эксклюзивную привязку unicast QUIC-сокета на `DEFAULT_QUIC_PORT` с fallback на порт `0` при конфликте (без `SO_REUSEADDR`, чтобы два wildcard-endpoint не делили один UDP-порт), проверять живость кэшированного `Connection` и при ошибке/таймауте `open_bi` или смене адреса/порта пира немедленно сбрасывать устаревшее соединение и переподключаться.
- [-] 2. **[P0] Восстановление видимости устройств и `Shares` после перезапуска приложения**:
   - В `src-tauri/src/lib.rs` и `LandaAndroidPlugin.kt`: запускать повторный `trigger_scan()` сразу после завершения запроса startup-разрешений (`ensureStartupPermissions`) и при `onResume` / `load` WebView.
   - В `src/features/shares/SharesView.tsx`: добавить кнопку ручного обновления списка устройств/шар, не терять выбранного пира при кратковременном мигании онлайна (или автоматически перезагружать `remoteRoots` при возвращении пира в онлайн), а также показывать понятное состояние при пустом списке шар или ошибке подключения.
   - Проверить сохранность SAF `content://` разрешений (`takePersistableUriPermission`) после перезапуска Android-приложения и добавить диагностику/тесты на переподключение QUIC после смены порта пира.
   - *(Проверено на физическом устройстве Xiaomi `25102PCBEG` (`POCO F8 Ultra`), Android 16 (`ro.build.version.release=16`), сборка `app-arm64-debug.apk`: через системный SAF-диалог выбрана и расшарена папка `LandaSafVerify` (`content://com.android.externalstorage.documents/tree/primary%3ADownload%2FLandaSafVerify`, `persistable=0x3 persisted=0x1`) с контрольным файлом `control.txt` (`33` байта, `SAF_FOLDER_RESTART_VERIFIED_2026`); после `am force-stop dev.landa.app` и холодного запуска (`LaunchState: COLD`) без повторного выбора через системный диалог доверенный ПК-пир `Windows-Verifier` (`192.168.0.234`) обнаружил устройство по LAN (`192.168.0.45:40406`), открыл корень `LandaSafVerify` (`browse_remote_roots`), получил список файлов (`browse_remote_dir` → `control.txt`), скачал файл (`download_remote_file` → содержимое `SAF_FOLDER_RESTART_VERIFIED_2026`) и скачал папку целиком (`download_remote_folder` → `file_count=1`, `total_bytes=33`).)*
