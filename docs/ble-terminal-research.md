# Исследование архитектуры BLE-терминала для Linux

Дата: 2026-10-24. Статус: этап 1 (исследование) завершён; ожидаются решения оператора (раздел 10).

Рабочие заметки с полными цитатами, SHA коммитов и номерами строк вынесены в приложения:

| Приложение | Тема |
|---|---|
| [research/A-mobile-ble-uart-apps.md](research/A-mobile-ble-uart-apps.md) | Мобильные BLE UART/terminal-приложения, «shell over BLE»-проекты |
| [research/B-terminal-engines.md](research/B-terminal-engines.md) | Терминальные движки и мобильные терминальные/SSH-клиенты |
| [research/C-linux-server.md](research/C-linux-server.md) | BlueZ D-Bus, L2CAP CoC, библиотеки, PTY/login/PAM, привилегии, пакетирование |
| [research/D-mobile-os-ble-apis.md](research/D-mobile-os-ble-apis.md) | Ограничения CoreBluetooth и Android BLE на уровне системных API |
| [research/E-security.md](research/E-security.md) | Pairing/bonding на headless-устройстве, модель угроз, SSH/Noise поверх BLE |

Обозначения: **[V]** — проверено по исходному коду или первичной документации (ссылка дана); **[S]** — вторичный источник (блог вендора, форум, измерения третьих лиц); **[H]** — гипотеза/оценка, требует эксперимента.

Исследованные ревизии (shallow clone): BlueZ `a0580dad` (5.87-dev), теги BlueZ 5.55/5.66–5.69/5.82; Linux `17bb0e2f`; bluer `57ec7045`; termux-app `8629e632`; connectbot `0bea3773`; connectbot/termlib `56934dd1`; connectbot/sshlib `9b80e3da`; SwiftTerm `c3436be3`; swift-nio-ssh `149f32ff`; xterm.dart `d35ba2cc`; blink `a90b4423`; libvterm `934bc2fb`; OpenSSH `26b1363c`; kable `ad88767b`; SimpleBluetoothLeTerminal `f322c06c`; nRF Toolbox (Android `c7f8e443`, iOS `d419fb24`); Bluefruit LE Connect; asteroid-ble-shell `36d19c6a`; linkr-bee `8ea74735`; sorah/bluetooth-getty `c98973e9` и др. (полный список — в приложениях).

---

## 1. Краткое резюме

1. **Готового решения нет.** Ни одно найденное open-source мобильное приложение не делает полноценную VT/xterm-эмуляцию поверх BLE: все BLE UART-клиенты (nRF Toolbox, Bluefruit, SimpleBluetoothLeTerminal и др.) — это лог строк с полем ввода [V, прил. A §1]. Серверные «shell over NUS» для Linux существуют (pi-loop, ble_tty, asteroid-ble-shell, coBlue), но ни у одного нет канала изменения размера терминала, управления потоком и мобильного клиента; два из них раздают root-shell без аутентификации [V, прил. C §4].
2. **Зрелые компоненты для сборки есть, и они удачно стыкуются.** Терминальные движки с транспорт-независимым API: **SwiftTerm** (iOS, MIT) и **ConnectBot termlib** / Termux terminal-emulator (Android, Apache-2.0) [V, прил. B]. SSH-библиотеки с подключаемым байтовым транспортом: **ConnectBot sshlib** (`Transport`/`TransportFactory`) и **SwiftNIO SSH** (работает поверх любого NIO Channel) [V, прил. E §4.2]. На сервере: `sshd -i` работает поверх произвольного потока stdin/stdout, это штатно тестируется в регрессии OpenSSH [V].
3. **Ключевой вывод исследования:** оптимальная архитектура — **BLE как «надёжная труба байтов» + SSH-протокол поверх неё**: на сервере per-connection `sshd -i` (PAM, PTY, controlling TTY, window-change, учёт сессий, host-ключи — всё из коробки), на клиентах — существующие пары «терминальный движок + SSH-библиотека», которые уже интегрированы друг с другом в ConnectBot (Android) и в демо-приложении SwiftTerm (iOS). Собственный код сводится к: (а) BLE-демону на Linux с GATT-сервисом и кредитным управлением потоком, (б) BLE-транспорту для sshlib/SwiftNIO SSH на клиентах. Это максимизирует повторное использование и одновременно решает главную проблему безопасности headless-устройства (BLE-pairing без дисплея не защищает от MITM; SSH host key pinning — защищает).
4. **Транспорт:** базовый — **собственный GATT-сервис** (write-without-response вход, notify выход, in-band кадрирование с кредитами). Сырой NUS непригоден как основной протокол (нет resize, нет backpressure, нет служебного канала). **L2CAP CoC** технически лучше на Linux (реальный credit-based backpressure, per-client сокеты, без привилегий), но совместимость iOS/Android ↔ BlueZ CoC публично не подтверждена, а поведение CoC в фоне на iOS не документировано — поэтому CoC планируется как опциональное ускорение после эксперимента, с откатом на GATT.
5. **Главные технические ограничения:** (а) в BlueZ нет backpressure для уведомлений (неограниченная очередь `att->write_queue`), а через AcquireWrite write-without-response молча теряются при EAGAIN — нужен свой кредитный протокол [V]; (б) BlueZ 5.66 (Debian/RPi OS Bookworm) не умеет per-device fd для AcquireNotify/AcquireWrite — на нём только один клиент на характеристику; с 5.69 (Trixie: 5.82) — per-device [V]; (в) на iOS нельзя задать MTU, интервал соединения ≥ 15 мс, нельзя программно инициировать pairing [V]; (г) на Android — одна GATT-операция в полёте [V].
6. **Сервер не требует** изменений `main.conf`, `--experimental`, root для BLE-части и не монополизирует адаптер: регистрация GATT/рекламы через D-Bus BlueZ доступна непривилегированному пользователю и исчезает при остановке процесса [V].
7. **Рекомендуемый MVP** проверяется без мобильных приложений: BLE-демон на Pi + десктопный клиент-«ProxyCommand» → обычный `ssh -o ProxyCommand=...` даёт полноценный терминал (vim/htop/tmux) через BLE уже на первом этапе. Затем — форк ConnectBot (Android) и приложение на SwiftTerm + SwiftNIO SSH (iOS).
8. Это расходится с буквой ТЗ в одном пункте: вместо `agetty`/`login` сессию поднимает `sshd` (который сам использует PAM и стандартный PTY). Альтернатива с `login`/PAM и собственным протоколом проработана (раздел 7.1) и остаётся реализуемой; выбор вынесен оператору (вопрос 1).

---

## 2. Найденные проекты

Активность — по `gh api` на 2026-10-24. «Пригодность»: ★★★ — брать как основу, ★★ — использовать частично/как библиотеку, ★ — только как справка, ✗ — не использовать.

### 2.1 Мобильные клиенты и BLE-библиотеки

| Проект | Платформа | Лицензия [V] | Активность | Что внутри | Пригодность |
|---|---|---|---|---|---|
| [connectbot/connectbot](https://github.com/connectbot/connectbot) | Android, Kotlin/Compose | Apache-2.0 | 3.5k★, push 2026-10-23 | Полноценный SSH-клиент: termlib + sshlib, хосты, extra-keys, фоновый сервис; `AbsTransport`; sshlib уже используется с нестандартным `transportFactory` для jump-хостов (`transport/SSH.kt:976-1011`) | ★★★ основа Android-клиента (форк + BLE-транспорт) |
| [connectbot/termlib](https://github.com/connectbot/termlib) | Android, Compose + libvterm (JNI) | Apache-2.0 + MIT | 28★, 0.3.x, активен | `writeInput` / `onKeyboardInput` / `onResize` — без привязки к транспорту (`TerminalEmulator.kt:82-275`) | ★★★ (через ConnectBot) |
| [connectbot/sshlib](https://github.com/connectbot/sshlib) | Kotlin/JVM | Apache-2.0 | активен, 0.6.x | `interface Transport` «any byte stream transport» (`transport/Transport.kt:22-63`), `TransportFactory` (`:50-58`) | ★★★ |
| [termux/termux-app](https://github.com/termux/termux-app) terminal-emulator / terminal-view | Android, Java | Библиотеки Apache-2.0 (по `LICENSE.md`; в подкаталогах нет своих файлов лицензии), остальное GPLv3; `ExtraKeysView` — GPLv3 | 63k★, push 2026-09 | Лучший IME, Unicode 15; `TerminalSession` — `final` и привязан к JNI/PTY (`TerminalSession.java:31,103-131`) | ★★ запасной движок (патч ~150–300 строк) |
| [migueldeicaza/SwiftTerm](https://github.com/migueldeicaza/SwiftTerm) | iOS/macOS, Swift | MIT | 1.7k★, v1.99.1 2026-10-21 | `feed(byteArray:)`, делегат `send(data:)`, `sizeChanged`; панель Esc/Ctrl; Unicode 17; esctest; демо SSH-терминала на swift-nio-ssh (`TerminalApp/iOSTerminal/UIKitSshTerminalView.swift`); на нём построены La Terminal и Secure ShellFish | ★★★ основа iOS-клиента |
| [apple/swift-nio-ssh](https://github.com/apple/swift-nio-ssh) | Swift | Apache-2.0 | Apple, активен | `NIOSSHHandler` с `ByteBuffer` вход/выход (`NIOSSHHandler.swift:120-124`), тесты на in-memory каналах; curve25519, Ed25519/ECDSA, AES-GCM; pinning через делегат | ★★★ |
| [JuulLabs/kable](https://github.com/JuulLabs/kable) | Kotlin Multiplatform | Apache-2.0 | 1.2k★, активен | Корректный flow control WNR на обеих ОС (`CBPeripheralCoreBluetoothPeripheral.kt:213-229`, `BluetoothDeviceAndroidPeripheral.kt:195-211`) | ★★ (опция общего BLE-слоя; справочник паттернов) |
| [nordicsemi/Kotlin-BLE-Library](https://github.com/nordicsemi/Kotlin-BLE-Library), [Android-BLE-Library](https://github.com/nordicsemi/Android-BLE-Library) | Android | BSD-3 | активны | Очереди запросов, MTU, split | ★★ альтернатива BLE-слоя на Android |
| [nordicsemi/IOS-CoreBluetooth-Mock](https://github.com/nordicsemi/IOS-CoreBluetooth-Mock) | iOS | BSD-3 | активен | Мок CoreBluetooth для тестов/симулятора | ★★ для тестов |
| [kai-morich/SimpleBluetoothLeTerminal](https://github.com/kai-morich/SimpleBluetoothLeTerminal) | Android | MIT | 345★, 2026-04 | Очередь записи по `onCharacteristicWrite` (`SerialSocket.java:349-421`), кредиты Telit TIO (`:506-645`); UI — лог строк | ★ справка (GATT-машина состояний, кредиты) |
| [nordicsemi/Android-nRF-Toolbox](https://github.com/nordicsemi/Android-nRF-Toolbox), [IOS-nRF-Toolbox](https://github.com/nordicsemi/IOS-nRF-Toolbox) | Android / iOS | BSD-3 | активны | UART-профиль: чат-лог; iOS отбрасывает не-UTF-8 пакеты и обрезает пробелы (`UARTViewModel.swift:172-175`) | ★ справка; как терминал ✗ |
| Adafruit Bluefruit LE Connect ([iOS](https://github.com/adafruit/Bluefruit_LE_Connect_v2), [Android](https://github.com/adafruit/Bluefruit_LE_Connect_Android_V2)) | iOS / Android | MIT | iOS — 2024, Android — архив | WNR в цикле без `canSendWriteWithoutResponse` (`BlePeripheral.swift:506-513`) — антипаттерн | ✗ |
| Serial Bluetooth Terminal (Kai Morich) | Android | проприетарная | Play Store | «line-oriented terminal» | ✗ (закрыт) |
| [johnhowe/BlueTerm](https://github.com/johnhowe/BlueTerm) | Android | нет LICENSE | 2014 | Единственный найденный мобильный VT100 поверх Bluetooth — но Classic SPP | ✗ (история) |
| [TerminalStudio/xterm.dart](https://github.com/TerminalStudio/xterm.dart) | Flutter | MIT | последний релиз 2024-02, 109 issues | API на `String`, Unicode 11 | ✗ (замедляется) |
| flutter_blue_plus | Flutter | **собственная лицензия** (коммерческая для for-profit, телеметрия при сборке) | активен | — | ✗ (лицензия); альтернатива universal_ble (BSD-3) |
| [blinksh/blink](https://github.com/blinksh/blink) | iOS | GPL-3.0 | 7k★ | hterm в WKWebView, нативный транспорт | ★ справка; форк для App Store — юридический риск |
| xterm.js (в WebView, не Web Bluetooth) | обе | MIT | 21k★, 6.0 | Лучшая эмуляция; открытые issues по мобильному вводу (#5377, #3600, #3727, #5108) | ★ запасной вариант общего UI |
| libvterm, alacritty_terminal, termwiz, libghostty-vt | C / Rust / Zig | MIT / Apache-2.0 / MIT | — | Только движки, без рендерера/IME; libghostty-vt без стабильного API | ✗ для этой задачи |

### 2.2 Серверная часть Linux и прецеденты

| Проект | Что это | Лицензия [V] | Активность | Вывод | Пригодность |
|---|---|---|---|---|---|
| [bluez/bluer](https://github.com/bluez/bluer) | Rust: BlueZ D-Bus (GATT server, advertising, agent) + L2CAP-сокеты | BSD-2-Clause | v0.17.4, 2026-08 | AcquireWrite/AcquireNotify как IO (`gatt/local.rs:902-962`), адрес устройства и MTU в запросах, L2CAP `StreamListener` с `set_security`, MTU | ★★★ основа демона |
| [kevincar/bless](https://github.com/kevincar/bless) | Python GATT server | MIT | 2026-04 | Работает, AcquireNotify есть; зависит от dbus-next (не обновлялся с 2024-06) | ★★ для прототипов |
| [ukBaz/python-bluezero](https://github.com/ukBaz/python-bluezero) | Python GATT server | MIT | 2025-03 | Нет Acquire* | ★ |
| dbus-fast | Python D-Bus | MIT | активен | Основа для своего Python-сервера | ★★ |
| tinygo-org/bluetooth | Go | BSD-3-подобная | активен | Не сообщает, какое устройство записало (`gatts_linux.go`) | ✗ для multi-client |
| go-ble/ble, @abandonware/bleno, BTstack | — | — | — | Захватывают HCI (user channel) / требуют остановить bluetoothd | ✗ (монопольный доступ) |
| [radxa/linkr-bee](https://github.com/radxa/linkr-bee) | ESP32 (Zephyr) мост UART-консоли → BLE/Wi-Fi + клиенты | Apache-2.0 | активен | Не Linux-сервер. Полезно: надёжный протокол с seq/индикациями, документированная потеря данных при переполнении очередей (`docs/BRIDGE_QUEUE_FINDINGS.md`), одно соединение, нет resize | ★ справка; вариант «внешний мост» |
| [sorah/bluetooth-getty](https://github.com/sorah/bluetooth-getty) | Classic SPP → per-connection systemd unit с agetty | MIT | — | Ближайший прецедент упаковки: .deb, hardened unit, PTY-прокси из-за `vhangup()` в login (`src/session_proxy.rs`) | ★★ образец упаковки/systemd |
| [monperrus/asteroid-ble-shell](https://github.com/monperrus/asteroid-ble-shell) | NUS-shell на AsteroidOS | GPL-3.0 | 2026-09 | Лучшая модель безопасности среди прецедентов (auth-флаги, «arm»-окно, пиннинг устройства, эскалация SIGHUP→TERM→KILL); PTY 80×24 без resize; без PAM | ★ справка |
| [VishovVladimir/ble_tty](https://github.com/VishovVladimir/ble_tty), [jonasneves/pi-loop](https://github.com/jonasneves/pi-loop) | NUS ↔ root bash на Pi | нет лицензии | 2026 | Без аутентификации и шифрования; 20-байтные чанки без flow control | ✗ (антипример) |
| [cocoahuke/coBlue](https://github.com/cocoahuke/coBlue) | BlueZ C-демон + macOS-клиент | MIT | 2018 | iOS-клиент так и не написан | ✗ |
| [Jakeler/ble-serial](https://github.com/Jakeler/ble-serial) | Десктоп: BLE UART → виртуальный serial | MIT | 2024-12 | Полезен как тестовый клиент | ★★ для тестов |
| Zephyr/NCS `CONFIG_SHELL_BT_NUS` | MCU-shell поверх NUS | Apache-2.0 | — | Самый массовый «shell over BLE», но MCU, не Linux | ★ |
| `rfcomm watch … getty` | Classic Bluetooth console | — | — | iOS не может использовать RFCOMM/SPP без MFi | ✗ |

**Внешний UART-мост (HM-10/ESP32 → getty):** документированного проекта «телефон → BLE-мост → Linux getty → полноэкранные приложения» не найдено [V: отсутствие в поиске]. Мост добавляет вторую границу управления потоком и не передаёт размер окна; оправдан только для хостов без собственного BLE (тогда linkr-bee — лучшая основа).

---

## 3. Сравнение BLE-транспортов

### 3.1 Ограничения платформ

| Параметр | iOS (CoreBluetooth) | Android | Linux/BlueZ (Pi) |
|---|---|---|---|
| ATT MTU | Нельзя запросить; ОС выбирает сама (наблюдалось 185/247/527 в зависимости от модели/версии). Размер чанка — `maximumWriteValueLength(.withoutResponse)` [V ADG §58.11; S] | `requestMtu`; с Android 14 всегда 517 при первом вызове [V] | `ExchangeMTU = 517` по умолчанию [V] |
| Макс. значение атрибута | 512 | 512 (`GATT_MAX_ATTR_LEN`) [V AOSP] | Уведомление длиннее MTU−3 **молча обрезается** (`shared/gatt-server.c:1828`) [V] |
| DLE / 2M PHY | Автоматически; периферия должна делать LL Length Update до MTU exchange [V ADG]; PHY — по инициативе периферии [S] | `setPreferredPhy` (API 26) [V] | Контроллер CYW43455 — проверить [H] |
| Интервал соединения | Мин. 15 мс, кратно 15; 11.25 мс только при HID; запрашивает периферия [V ADG §58.6] | `requestConnectionPriority(HIGH)` = 11.25–15 мс [V AOSP config.xml] | Запрос параметров со стороны периферии через D-Bus не выставлен [H] |
| Управление потоком на запись (клиент → сервер) | `canSendWriteWithoutResponse` + `peripheralIsReady(toSendWriteWithoutResponse:)` [V] | Одна операция в полёте; `onCharacteristicWrite` приходит и для WNR, задерживается при congestion [V AOSP `GattService`] | AcquireWrite: запись подтверждается до чтения приложением; WNR **молча отбрасывается** при EAGAIN (`gatt-database.c:3348-3355`) [V] |
| Backpressure уведомлений (сервер → клиент) | Нет API [V] | Нет API [V] | Очередь `att->write_queue` неограниченна (`shared/att.c:1681-1687`) [V] |
| L2CAP CoC | iOS 11+ `openL2CAPChannel(PSM)`; PSM из характеристики `ABDD3056-28FA-441D-A470-55A75A52553A` [V]; баг с несколькими каналами на одно устройство [S] | API 29+ `createL2capChannel` (требует bond) / `createInsecureL2capChannel` [V] | Сокет `SOCK_SEQPACKET`, PSM 0x80–0xFF без привилегий (`l2cap_sock.c:69-80`), credit-based FC в ядре [V] |
| Фон | `bluetooth-central`: уведомления будят приложение (~10 с); state restoration; не перезапускается после force-quit/выключения BT (QA1962) [V]; CoC в фоне не документирован [H] | FGS `connectedDevice` (Android 14+) [V]; CompanionDeviceManager [V] | — |
| Pairing | Нельзя инициировать из приложения; триггер — ATT Insufficient Authentication/Encryption [V ADG §58.10] | `createBond()` или неявно при ошибке аутентификации [V] | IO capability задаёт default agent, на уровне всего адаптера [V `adapter.c:9508-9535`] |

### 3.2 Варианты транспорта

| Критерий | Сырой NUS | Собственный GATT-профиль (рекомендуется) | L2CAP CoC |
|---|---|---|---|
| Совместимость со сторонними UART-приложениями | Да (только построчный режим) | Нет (опционально — отдельный NUS-сервис совместимости) | Нет |
| Resize/сигналы/служебные сообщения | Нет (только in-band костыли) | Да (in-band кадрирование) | Да |
| Backpressure сервер → клиент | Нет | Кредиты на уровне приложения | Нативно, кредиты ядра; на ядрах ≥ 6.10 ещё и по свободному месту приёмного буфера (коммит `ce60b9231b66`) [V] |
| Backpressure клиент → сервер | ОС клиента + риск потерь в bluetoothd | ОС клиента + кредиты сервера | Нативно |
| Мульти-клиент на BlueZ 5.66 | Нет (широковещательная рассылка) | Нет (только один клиент) | Да (сокет на `accept()`) |
| Мульти-клиент на BlueZ ≥ 5.69 | Частично | Да (per-device fd + HUP при отключении) | Да |
| Пропускная способность | 30–100 KB/s [S] | 30–100 KB/s [S] | WWDC17: до ~49 KB/s на iOS (L2CAP+DLE+15 мс); Nordic: прирост над GATT незначителен [S] |
| Подтверждённая совместимость iOS/Android ↔ BlueZ | Да | Да | **Не найдено публичного подтверждения** [S] |
| Работа в фоне iOS | Да | Да | Не документировано [H] |
| Сложность | Минимальная | Небольшая (~кадр 1–3 байта + кредиты) | Средняя (+ GATT для обнаружения PSM всё равно нужен) |

**Вывод:** собственный GATT-сервис — базовый и обязательный транспорт; L2CAP CoC — опциональное улучшение после эксперимента E2; сырой NUS — только как отдельный необязательный режим совместимости.

Для терминала пропускная способность не узкое место: полная перерисовка экрана 200×60 с SGR — 12–40 KB (0.2–1 с в худшем случае), инкрементальные обновления vim/htop много меньше [H]. Определяющие факторы — **задержка** (эхо нажатия ≈ 2 интервала соединения: ~30–60 мс) и **корректный backpressure** при `cat bigfile` [S/H].

---

## 4. Клиентская часть Android и iOS

### 4.1 Что показал код существующих BLE UART-приложений

- Все — лог сообщений, а не терминал: нет парсера CSI, сетки ячеек, альтернативного экрана, Ctrl/Esc/стрелок [V, прил. A §1].
- Типовая ошибка — декодирование UTF-8 по каждому уведомлению отдельно (`new String(data)` в SimpleBluetoothLeTerminal `TerminalFragment.java:254`, `String(it)` в nRF Toolbox Android `UARTManager.kt:44`): многобайтовые символы на границе пакетов портятся. Терминальный клиент должен отдавать **сырые байты** движку.
- Полезные паттерны: очередь записи по `onCharacteristicWrite` (Android), gating WNR по `canSendWriteWithoutResponse` (iOS), кредиты Telit TIO как образец backpressure [V].
- Вывод: ни одно UART-приложение не годится как основа форка; брать надо терминальные/SSH-клиенты.

### 4.2 Терминальные движки (по исходникам)

| Движок | Вход байтов | Выход байтов | Resize | xterm-уровень | Ширина Unicode | Ввод |
|---|---|---|---|---|---|---|
| ConnectBot termlib | `writeInput` (`TerminalEmulator.kt:107`) | `onKeyboardInput` callback (`:252-275`) | `onResize(cols, rows, px)` | truecolor, мышь, alt screen, bracketed paste, OSC 52 | **старая таблица Markus Kuhn** — риск расхождения с glibc для emoji [H] | TYPE_NULL + compose-режим (`ImeInputView.kt`) |
| Termux terminal-emulator | `TerminalEmulator.append` (`:688`) | `TerminalOutput.write` | `TerminalSession.updateSize` → JNI (нужен патч) | то же + sixel | Unicode 15 | лучший IME |
| SwiftTerm | `feed(byteArray:)` (`AppleTerminalView.swift:1264`) | `send(source:data:)` (`TerminalViewDelegate.swift:73`) | `sizeChanged` (`:57`) | то же + Kitty graphics/keyboard; esctest | **Unicode 17** | `UITextInput`, accessory bar с Esc/Ctrl (`iOSAccessoryView.swift:22`) |

Все три принимают и отдают **сырые байты** и сообщают размер как (cols, rows) — это прямо ложится и на собственный протокол, и на SSH `window-change`.

### 4.3 Готовые связки «движок + SSH»

- **Android — ConnectBot** (Apache-2.0) уже связывает termlib и sshlib (`service/TerminalBridge.kt:490-530`: `onKeyboardInput`/`onResize` → `WriteData`/`SetDimensions`). sshlib принимает произвольный транспорт через `SshClientConfig.transportFactory`, и **ConnectBot уже использует это** для цепочек jump-хостов: `transport/SSH.kt:976-1011` (connectbot `0bea3773`) [V, проверено вручную]. Значит, BLE-транспорт для SSH в ConnectBot — это реализация `org.connectbot.sshlib.transport.Transport` поверх GATT + новый тип хоста (`ble://…`) в UI. Попутно бесплатно: менеджер хостов, ключи, extra-keys (`ui/components/TerminalKeyboard.kt`, Apache-2.0), фоновый сервис, уведомления.
- **iOS — SwiftTerm** содержит демо SSH-терминала на swift-nio-ssh (`TerminalApp/iOSTerminal/UIKitSshTerminalView.swift`, SwiftTerm `c3436be3`): `send` → SSH-канал, `sizeChanged` → window-change, данные канала → `feed`. Для BLE нужно заменить TCP-bootstrap NIO на канал поверх CoreBluetooth [H: умеренный объём; SwiftNIO SSH в своих тестах работает поверх in-memory `EmbeddedChannel` — `EndToEndTests.swift:26-44`]. Готового iOS-приложения-форка нет: Blink — GPLv3, La Terminal / Secure ShellFish — закрытые. iOS-клиент — новое приложение из зрелых компонентов.

### 4.4 Общий код между платформами

Изучены: Flutter (xterm.dart + universal_ble), WebView + xterm.js с нативным BLE-мостом, libvterm/alacritty_terminal через FFI, KMP. Вывод [H, прил. B §4]: **нативный UI на каждой платформе**, общий — только спецификация BLE-кадрирования с тест-векторами (≈500 строк реализации на платформу) или, опционально, Kable/KMP для BLE-слоя. Общий терминальный движок не окупается: рендерер, IME, выделение текста и accessibility всё равно платформенные, а мобильный ввод в xterm.js проблемный (открытые issues).

### 4.5 Фон и переподключение

- iOS: режим `bluetooth-central` держит GATT-соединение, уведомления будят приложение; `connect()` не имеет таймаута и подходит для автопереподключения; iOS 17 — `EnableAutoReconnect`. После force-quit или выключения Bluetooth система приложение не перезапускает (QA1962) [V].
- Android: открытая сессия — только в foreground service типа `connectedDevice`; для фонового переподключения — `autoConnect=true` или CompanionDeviceManager presence [V].
- Поскольку по ТЗ сессия завершается при разрыве BLE, «переподключение» = новое соединение + новый вход. Политика сохранения сессий (tmux на сервере или на уровне демона) — отдельное решение (вопрос 6). Детальную политику фона ТЗ откладывает.

---

## 5. Серверная часть Linux/BlueZ

### 5.1 Взаимодействие с BlueZ

- **Только D-Bus API BlueZ** (`GattManager1.RegisterApplication`, `LEAdvertisingManager1.RegisterAdvertisement`) — без монопольного доступа к адаптеру; регистрации привязаны к D-Bus-соединению и исчезают при остановке процесса (`src/advertising.c:225`) — удобно для удаляемого пакета [V].
- **Политика D-Bus** upstream и в Trixie разрешает любому локальному пользователю вызывать методы `org.bluez` (`src/bluetooth.conf:26-28`); в Bookworm Debian-патч дополнительно разрешает группе `bluetooth` [V]. Демон может работать от отдельного непривилегированного пользователя (при необходимости — член группы `bluetooth`). Следствие: bluetoothd не является границей безопасности между локальными пользователями — граница аутентификации — SSH/PAM.
- **`--experimental`, `--compat` и правка `main.conf` не нужны** ни для GATT-сервера, ни для Acquire*, ни для рекламы, ни для L2CAP [V, прил. C §1.8].
- **AcquireWrite/AcquireNotify** (fd-путь, `doc/org.bluez.GattCharacteristic.rst:156-246`) предпочтительнее `WriteValue`/`PropertiesChanged`: меньше D-Bus-трафика, MTU приходит в опциях; сокет нужно создавать `SOCK_SEQPACKET|SOCK_NONBLOCK` (как в `client/gatt.c:2568` и bluer). bluer отнимает 5 байт от MTU из-за обрезки данных в BlueZ (`bluer/src/gatt/mod.rs:358-364`) [V].
- **Версии BlueZ** [V]: Debian/RPi OS Bookworm — 5.66, Trixie — 5.82. Per-device AcquireWrite/AcquireNotify (отдельные fd на клиента, HUP при отключении) появились в коммите `8eb1dee87e01` (5.69). На 5.66: один fd на характеристику, уведомления рассылаются всем подписчикам, записи разных устройств сливаются; отключение нужно отслеживать через `Device1.Connected`.
- **Реклама** останавливается при входящем соединении и автоматически возобновляется ядром только после разрыва (`hci_event.c:5893`, `:3574`); для второго клиента демон должен переподнять рекламу, поддержка состояния контроллером CYW43455 не проверена [V/H].
- **Сосуществование:** GATT-сервис и реклама добавляются рядом с другими сервисами; L2CAP-слушатель на динамическом PSM не конфликтует с bluetoothd [V]. Затрагивают весь адаптер только настройки pairing (IO capability, Pairable/Bondable) — поэтому рекомендуемая схема не требует их менять (раздел 6.5).

### 5.2 Управление потоком на сервере

- Нужен **прикладной кредитный протокол** в обе стороны [V-обоснование: неограниченная очередь уведомлений; потеря WNR в AcquireWrite при EAGAIN].
- Направление сервер → клиент: клиент выдаёт кредиты в байтах по мере потребления; демон не читает из сессии (stdout `sshd -i` или PTY master), когда кредиты кончились, — тогда штатно блокируется писатель (ядро PTY / сокет). Это даёт естественный backpressure до `cat bigfile`.
- Направление клиент → сервер: клиенты ограничены своими ОС (одна запись в полёте / `canSendWriteWithoutResponse`), но чтобы bluetoothd не терял WNR при медленном демоне, демон выдаёт клиенту окно приёма (кредиты), соответствующее свободному месту своего буфера.
- Механизм с повторной передачей и номерами последовательностей (как в linkr-bee) **не нужен**: link layer BLE надёжен (ACK/CRC), потери возникают только в программных очередях, которые закрываются кредитами; а при разрыве сессия всё равно завершается [H, обосновано].

### 5.3 Сессия: PTY, PAM, controlling TTY, завершение

Два варианта, общие для обеих архитектур:

**(a) `sshd -i` (рекомендуемый)** — демон передаёт поток в per-connection `sshd -i`:
- `sshd -i` (inetd-режим) поддерживается, в т.ч. после разделения sshd/sshd-session (`sshd.c:1400-1402, 1865-1870`); регрессия OpenSSH сама гоняет сервер через `ProxyCommand … sshd -i` (`regress/test-exec.sh:887`) [V].
- sshd сам делает PAM (`UsePAM yes`), выделяет PTY, `setsid`/`TIOCSCTTY`, utmp/wtmp/lastlog, `window-change` → `TIOCSWINSZ`, SIGHUP при закрытии; `pam_systemd` создаёт logind-сессию [V для механизмов, прил. C §5].
- Отдельный конфиг (`sshd -i -f /etc/ble-term/sshd_config`) — не затрагивает сетевой sshd, работает даже если `ssh.service` выключен. В Debian Bookworm (openssh-server 9.2) `ssh.socket` — `Accept=no`, шаблона `ssh@.service` нет [V, локально] — поэтому пакет ставит свой шаблон.
- Привилегии: запуск через systemd socket activation — `ble-term-session.socket` (`ListenStream=/run/ble-term/session.sock`, `Accept=yes`, `SocketUser=_bleterm`, `SocketMode=0600`) + `ble-term-session@.service` (`ExecStart=-/usr/sbin/sshd -i -f …`, `StandardInput=socket`). **Наш привилегированный код — ноль строк**; демон, разбирающий радиоввод, работает без root [H, стандартный приём systemd].
- Завершение: разрыв BLE → демон закрывает unix-сокет → sshd получает EOF → закрывает канал и PTY → SIGHUP группе процессов сессии, `pam_close_session`; юнит останавливается, его cgroup убивается [V механизм; H для деталей в связке].

**(b) `login`/`agetty` на PTY (вариант буквального ТЗ):**
- Тот же socket-activated root-юнит, но запускает маленький PTY-хелпер: `openpty` → `setsid` → `TIOCSCTTY` → `login -p -h ble:<addr>` (как shellinabox) или `agetty -` [V, прил. C §5.1]; master-fd передаётся демону через `SCM_RIGHTS` либо хелпер сам проксирует.
- Обязательно учесть: util-linux `login` делает `vhangup()` (`login.c:600-622`), master временно отдаёт EIO — прокси должен это переживать (как `systemd/src/shared/ptyfwd.c:905-912` и bluetooth-getty) [V].
- Resize — собственным служебным сообщением → `TIOCSWINSZ` на master.
- Этот хелпер — наш собственный root-код (~200–400 строк, C или Rust) [H].

**Для обоих вариантов:** `pam_systemd` переносит процессы пользователя в `session-N.scope`, поэтому `KillMode=control-group` нашего юнита **не убивает** shell-дерево; поведение по умолчанию как у ssh (SIGHUP; tmux/nohup выживают). Для строгого «убить всё» — `org.freedesktop.login1.Manager.TerminateSession` [V механизм; вопрос 6].

### 5.4 Библиотеки и язык демона

Рекомендуется **Rust + bluer** (BSD-2): единственная найденная библиотека, где одновременно есть GATT-сервер через D-Bus с AcquireWrite/AcquireNotify, сведения об устройстве и MTU в запросах, L2CAP-сокеты с уровнем безопасности и асинхронность; на выходе статический бинарник с минимумом зависимостей [V, прил. C §3]. Альтернатива — Python + dbus-fast (быстрее прототип, больше runtime-зависимостей; bless завязан на неподдерживаемый dbus-next).

### 5.5 Пакет

`.deb` (бинарник демона; `ble-term.service` от `_bleterm`/`DynamicUser`, `After=bluetooth.service`, `PartOf=bluetooth.service` для перерегистрации при рестарте bluetoothd; `ble-term-session.socket` + `ble-term-session@.service`; `sysusers.d`; `/etc/ble-term/sshd_config` и конфиг демона). Зависимости: `bluez`, `openssh-server` (только бинарник sshd; сетевой сервис может оставаться выключенным). Удаление: остановка юнитов — регистрации в BlueZ исчезают; созданные bond'ы (если включён pairing) остаются в `/var/lib/bluetooth` — документировать. Образец hardened-юнита — sorah/bluetooth-getty (`RestrictAddressFamilies=AF_UNIX AF_BLUETOOTH`, `ProtectSystem=strict` и т.д.) [V].

---

## 6. Предлагаемая архитектура

### 6.1 Компоненты

```
 ┌──────────────── Android ────────────────┐      ┌────────────────── iOS ──────────────────┐
 │ ConnectBot (форк, Apache-2.0)            │      │ Новое приложение                         │
 │  termlib (libvterm) ── TerminalBridge    │      │  SwiftTerm TerminalView                  │
 │        │ bytes / onResize                │      │        │ send / sizeChanged / feed        │
 │  sshlib SshClient (host-key pinning,     │      │  SwiftNIO SSH (NIOSSHHandler,             │
 │        pubkey/password, window-change)   │      │        host-key pinning)                 │
 │        │ Transport (read/write/close)    │      │        │ custom NIO Channel               │
 │  BleTransport ── BleStream (кредиты,     │      │  BleStream (кредиты, кадры)              │
 │        кадры) ── android.bluetooth GATT  │      │        └── CoreBluetooth GATT            │
 │  FGS connectedDevice, CDM                │      │  bluetooth-central background mode       │
 └────────────────────┬─────────────────────┘      └────────────────────┬─────────────────────┘
                      │            BLE (LE Secure Connections — опционально)                  │
                      └───────────────┬────────────────────────────────────┘
                                      │  GATT-сервис «BLE-Term»
                                      │   • Stream-In  : write-without-response (+write)
                                      │   • Stream-Out : notify
                                      │   • Info       : read (версия протокола, возможности)
                                      │   • L2CAP PSM  : read, UUID ABDD3056-… (этап 4)
 ┌────────────────────────────────────┼─────────────── Linux (Raspberry Pi) ─────────────────────┐
 │  bluetoothd (системный, без изменений конфигурации)                                           │
 │        ▲ D-Bus: GattManager1, LEAdvertisingManager1, AcquireWrite/AcquireNotify (fd)           │
 │  ble-term-daemon (Rust/bluer, пользователь _bleterm, без root)                                 │
 │     • реклама, GATT-приложение, учёт соединений (Device1.Connected / HUP fd)                   │
 │     • BleStream: кадрирование, кредиты, таймауты, лимит клиентов                               │
 │     • на каждое соединение: connect() в /run/ble-term/session.sock                             │
 │        ▼ AF_UNIX (поток байтов SSH)                                                            │
 │  systemd: ble-term-session.socket (Accept=yes) → ble-term-session@N.service (root)             │
 │        ExecStart=-/usr/sbin/sshd -i -f /etc/ble-term/sshd_config   StandardInput=socket        │
 │           └─ PAM (common-auth, pam_systemd, pam_faillock) → PTY /dev/pts/N → login shell        │
 └───────────────────────────────────────────────────────────────────────────────────────────────┘
```

### 6.2 Потоки данных

```
Нажатие клавиши (Android):
 termlib.onKeyboardInput(bytes) → SshClient channel data → sshlib шифрует (≈36–48 B)
 → BleTransport.write → BleStream: кадр DATA, ждёт кредит → writeCharacteristic(WNR)
 → onCharacteristicWrite → следующий кадр
 → bluetoothd → AcquireWrite fd → daemon: снять кадр, данные → unix socket → sshd stdin
 → PTY master → line discipline → приложение (vim)

Вывод (сервер → клиент):
 vim → PTY → sshd → unix socket → daemon читает, ТОЛЬКО если есть кредиты клиента
 → кадры ≤ MTU−5 → AcquireNotify fd → bluetoothd → notify
 → клиент: BleStream собирает поток, по мере потребления SSH-слоем шлёт CREDIT
 → sshlib/SwiftNIO SSH расшифровывает → termlib.writeInput / SwiftTerm.feed

Resize:
 termlib.onResize / SwiftTerm.sizeChanged → SSH "window-change" → sshd → TIOCSWINSZ → SIGWINCH

Разрыв BLE:
 HUP на per-device fd (BlueZ ≥5.69) или Device1.Connected=false (5.66) → daemon закрывает
 unix socket → sshd: EOF → закрыть PTY → SIGHUP сессии → pam_close_session → юнит завершён
```

### 6.3 Протокол BleStream (черновик, уточняется после экспериментов)

- Один GATT-сервис с собственным 128-битным UUID; две характеристики потока. Служебные сообщения — **in-band** (сохраняется порядок относительно данных, не зависит от планирования разных характеристик в ОС).
- Кадр = 1 байт типа + данные: `HELLO(version, caps, rx_window)`, `DATA(bytes)`, `CREDIT(u32 bytes)`, `CLOSE(reason)`, `PING/PONG`. Без номеров последовательностей (обоснование в 5.2).
- Перед первым `DATA` — обмен `HELLO`; сервер не открывает сессию (не дёргает `sshd`) до `HELLO` — защита от «пустых» подключений; таймаут неактивного/недоаутентифицированного соединения → `Device1.Disconnect()`.
- Размер кадра — по MTU, полученному в опциях AcquireNotify/AcquireWrite, минус запас bluer.
- Транспорт непрозрачен: внутри — SSH-поток (вариант 1) или терминальный поток с `RESIZE` (вариант 7.1). Это позволяет выбрать вариант сессии (вопрос 1), не меняя BLE-слой.

### 6.4 Мульти-клиент

MVP — **один активный клиент** (остальным `CLOSE(busy)` и отключение). Архитектура (per-connection состояние, отдельный `sshd -i` на соединение) мульти-клиент допускает: на BlueZ ≥ 5.69 через per-device fd, на 5.66 — только через L2CAP CoC. Плюс нужен эксперимент E3 (реклама при активном соединении на CYW43455).

### 6.5 Безопасность (рекомендуемая схема)

- **Аутентичность сервера:** SSH host key, отдельный для BLE-Term или системный; клиент пиннит отпечаток через QR-код/напечатанный fingerprint или TOFU при первом подключении. Это закрывает MITM при первом контакте и фишинг через поддельный «Pi» (BLESA-класс), что BLE-pairing на headless-устройстве не обеспечивает [V, прил. E §1, §3].
- **Аутентичность пользователя:** стандартный PAM через sshd (пароль) и/или публичный ключ из Android Keystore / iOS Secure Enclave; `pam_faillock`, `MaxAuthTries`, запрет root в `/etc/ble-term/sshd_config`.
- **Конфиденциальность:** шифрование SSH (curve25519 + AES-GCM/ChaCha20-Poly1305); `ObscureKeystrokeTiming` отключить (sshlib: `obscureKeystrokeTimingIntervalMs = 0`), иначе ~2 KB/s служебного трафика [V].
- **BLE-уровень по умолчанию — без обязательного pairing** (encrypt-флаги не ставятся): не нужен default agent, не меняются Pairable/Bondable/IO capability адаптера, нет системных диалогов pairing и проблем с «забытыми» bond'ами на iOS. Атака на канал до аутентификации SSH равна атаке на сетевой sshd — стандартная, хорошо защищённая поверхность.
- **Опциональное усиление (конфиг):** LE Secure Connections Just Works + default agent с «окном pairing» по кнопке/CLI (`RequestAuthorization`, `device.c:7972`) + `encrypt-*` флаги — снижает DoS/предаутентификационную поверхность и скрывает метаданные трафика [V механизм]. Статический passkey на Linux **невозможен** без патча ядра (ключ генерирует ядро, `smp.c:2805-2815`) и всё равно был бы слабым (CVE-2020-26558) [V].
- Вне зависимости от режима: «близость» не является фактором аутентификации (релей-атаки NCC 2022), автологин по bonded-устройству запрещён [V].

### 6.6 Критерии ТЗ

| Требование | Как закрывается |
|---|---|
| Только BLE, без IP | GATT/L2CAP; SSH — лишь протокол поверх байтового потока, сети нет |
| vim/htop/tmux/nmtui, xterm, Unicode, Ctrl, resize | termlib/SwiftTerm + SSH window-change |
| Повторное использование зрелого клиента | Форк ConnectBot; SwiftTerm + SwiftNIO SSH (готовая связка в демо SwiftTerm) |
| Стандартные PTY и PAM, системная аутентификация | sshd: PTY + PAM + logind; `login`/`agetty` — только в варианте 7.1 (вопрос 1) |
| Минимум вмешательства в BlueZ, сосуществование | Только D-Bus-регистрации, без правки `main.conf`, без захвата адаптера, без default agent по умолчанию |
| Установка/удаление как пакет | `.deb` + systemd-юниты, без постоянного состояния в bluetoothd |
| Завершение сессии при разрыве | EOF → sshd → SIGHUP; строгий режим — TerminateSession |
| Минимум привилегий | Демон без root; root только у sshd через socket activation |
| Без root/jailbreak на телефонах | Только публичные API (CoreBluetooth, android.bluetooth) |

---

## 7. Альтернативные архитектуры и причины отклонения

### 7.1 Собственный терминальный протокол + `login`/PAM (главная альтернатива — не отклонена окончательно)

BleStream с кадрами `DATA`/`RESIZE`/`SIGNAL`; на сервере PTY-хелпер с `login -p -h ble:<addr>`; клиенты подключают termlib/SwiftTerm к BLE напрямую (glue ~0.5–1 день на платформу [H]).

- **Плюсы:** буквально соответствует ТЗ (`agetty`/`login`); нет SSH-библиотек в клиентах; ~на 35–50 байт меньше на нажатие; не нужен `openssh-server`.
- **Минусы:** (1) безопасность целиком на BLE-pairing, а на headless-устройстве он либо Just Works (нет защиты от MITM, пароль login уходит открытым текстом внутри канала), либо требует показать passkey — без дисплея непрактично [V, прил. E §3]; добавление защиты на уровне приложения (Noise) = переизобретение SSH; (2) свой root-хелпер (PTY, vhangup/EIO, utmp, передача master-fd); (3) на Android теряется готовая интеграция ConnectBot с SSH — нужен свой «хост-тип» и транспорт для termlib; (4) нет аутентификации по ключам.
- **Статус:** запасной вариант; BLE-слой спроектирован так, что переход между 6 и 7.1 не меняет транспорт.

### 7.2 Сырой NUS + PTY (pi-loop, ble_tty, asteroid-ble-shell)
Нет resize, нет backpressure, нет служебного канала; совместим со сторонними UART-приложениями, но только в построчном режиме. **Отклонено** как основной вариант; возможен как отдельный выключенный по умолчанию режим совместимости (вопрос 7).

### 7.3 L2CAP CoC как единственный транспорт
Лучший backpressure и мульти-клиент на любой версии BlueZ, но нет подтверждённой совместимости iOS/Android ↔ BlueZ, не документирована работа в фоне iOS, баги iOS с несколькими каналами, фрагментация Android; GATT всё равно нужен для обнаружения PSM. **Отклонено как единственный**; планируется как улучшение (этап 4) после эксперимента E2.

### 7.4 Noise (или иной прикладной канал) + собственный протокол
Легче SSH (рукопожатие ~100–200 B), но придётся заново делать аутентификацию пользователя, PTY/resize, rekeying, аудит; библиотеки менее проверены. **Отклонено** — SSH даёт всё это зрелым кодом, а его накладные расходы для BLE приемлемы (рукопожатие ~3–8 KB, ~0.1–0.5 с [H]).

### 7.5 Общий кроссплатформенный терминальный UI (Flutter / WebView+xterm.js / libvterm-FFI / KMP)
xterm.dart замедлился и работает со строками; flutter_blue_plus — лицензионная ловушка; xterm.js — проблемы мобильного ввода; FFI-движки требуют своих рендереров (6–10+ человеко-недель). **Отклонено** в пользу нативных зрелых движков [H-оценки, прил. B §4].

### 7.6 Внешний BLE-UART-мост (ESP32/linkr-bee → serial getty)
Не требует BLE на хосте и изменений Linux, но: дополнительное железо, нет resize, вторая граница flow control, linkr-bee — одно соединение и документированные потери в очередях. **Отклонено** для референсной платформы (у Pi есть свой BLE); уместно как отдельный продукт для хостов без BLE.

### 7.7 Стеки с монопольным доступом к HCI (bleno, go-ble, BTstack)
Противоречат требованию сосуществования. **Отклонено.**

### 7.8 Bluetooth Classic (RFCOMM/SPP getty)
iOS-приложениям недоступен без MFi [V, MFi FAQ]. **Отклонено.**

### 7.9 Форк Termux / Blink
Termux-app — GPLv3 (используются только Apache-библиотеки движка); Blink — GPLv3, распространение форка через App Store юридически рискованно [H]. **Отклонено** как основа приложений.

---

## 8. Риски, неизвестные и необходимые эксперименты

| # | Риск / неизвестное | Влияние | Эксперимент |
|---|---|---|---|
| E1 | Реальные MTU, интервал соединения, задержка эха и пропускная способность GATT notify/WNR между Pi (CYW43455) и современными iPhone/Android; поведение bluetoothd при всплеске уведомлений | Отзывчивость, размер кредитного окна | Прототип BleStream на bluer: замер RTT эха, KB/s, памяти bluetoothd при `cat` с кредитами и без |
| E2 | iOS/Android ↔ BlueZ L2CAP CoC (без шифрования и с ним), инициирует ли iOS pairing при отказе по безопасности, CoC в фоне на iOS | Этап 4 (CoC) | `openL2CAPChannel` / `createInsecureL2capChannel` к bluer `StreamListener` |
| E3 | Несколько одновременных LE-соединений на CYW43455; переподнятие рекламы при активном соединении | Мульти-клиент | `btmgmt info`, LE supported states, два телефона |
| E4 | sshlib поверх BLE-`Transport` на Android (JVM 17 target, desugaring); SwiftNIO SSH поверх собственного Channel на CoreBluetooth; время рукопожатия (mlkem768x25519 vs curve25519) | Сроки клиентов | Минимальные PoC на обеих платформах |
| E5 | Расхождение ширины Unicode (старая таблица libvterm в termlib vs glibc Pi) | Искажения vim/htop с emoji/CJK | Тест-строки emoji/CJK в vim/htop/tmux; при необходимости обновить таблицу в termlib или перейти на движок Termux |
| E6 | Поведение iOS в фоне с открытой сессией (заморозка, частота пробуждений), Android FGS у OEM (Samsung/Xiaomi) | UX фона | Сценарии блокировки экрана/переключения приложений |
| E7 | `sshd -i` через socket activation: завершение по EOF, utmp/`who`, `pam_systemd`, поведение на OpenSSH 9.2 (Bookworm) и 10.x (Trixie) | Корректность сессий | Тест на обоих выпусках ОС |
| E8 | BlueZ 5.66: отсутствие HUP на fd при отключении, общий fd; сценарии «клиент пропал без disconnect» | Зависшие сессии | Тест с выключением BT на телефоне, уходом из зоны; supervision timeout |
| E9 | Плагин battery в bluetoothd вызывает повторные запросы pairing у iOS при подключении к Linux-периферии (сообщение Ditto) [S] | UX без pairing | Проверить на 5.66 и 5.82; если подтвердится — найти обход без правки системной конфигурации или документировать |
| E10 | Запрос параметров соединения (15 мс) и 2M PHY со стороны периферии без правки конфигурации BlueZ | Задержка на iOS | Проверить, что выбирает iOS по умолчанию; при необходимости — `hcitool`/mgmt-эквиваленты (без изменения `main.conf`) |

Прочие риски: termlib — пре-1.0 (0.3.x), API может меняться; termux terminal-emulator — лицензия Apache зафиксирована только в корневом `LICENSE.md`; App Store review — фоновый режим `bluetooth-central` нужно обосновать.

---

## 9. Этапы MVP и развития

| Этап | Содержание | Результат |
|---|---|---|
| 0. Эксперименты | E1, E4 (минимальные PoC), E7, E9; при наличии ресурса — E2 | Подтверждённые параметры кредитного окна и кадров, выбор CoC/не-CoC |
| 1. Сервер + десктоп-клиент (MVP) | `ble-term-daemon` (Rust/bluer): GATT-сервис, реклама, BleStream с кредитами, один клиент, socket-activated `sshd -i`; десктопный `ble-term-connect` (Rust/bluer как central) для `ssh -o ProxyCommand="ble-term-connect <addr>" user@pi`; `.deb` | Полноценный терминал (vim/htop/tmux/nmtui) через BLE с ноутбука; проверка всего серверного контура |
| 2. Android | Форк ConnectBot: `BleTransport` для sshlib, BLE-тип хоста (сканирование, CDM), FGS `connectedDevice`, пиннинг host key (QR/TOFU) | Android-клиент уровня ConnectBot |
| 3. iOS | Приложение на SwiftTerm + SwiftNIO SSH + CoreBluetooth BleStream, accessory bar, список устройств, пиннинг ключа, Keychain/Secure Enclave | iOS-клиент |
| 4. Развитие | L2CAP CoC с откатом на GATT; мульти-клиент (BlueZ ≥ 5.69 / CoC); опциональный BLE pairing с окном по кнопке; QR-провижининг; переподключение в фоне; (опционально) NUS-режим совместимости; вариант 7.1 при выборе | Полный продукт |

Вариант 7.1 (login/PAM без SSH), если оператор его выберет, меняет этапы 1–3 так: в этапе 1 добавляется root PTY-хелпер и кадр `RESIZE`; в этапах 2–3 termlib/SwiftTerm подключаются к BleStream напрямую (без SSH-библиотек), Android-клиент — собственное приложение на termlib или урезанный форк ConnectBot.

---

## 10. Решения, требующие участия оператора

1. Слой сессии: SSH поверх BLE (`sshd -i`) или собственный протокол с `login`/`agetty` (отступление от буквы ТЗ vs безопасность и повторное использование).
2. Уровень безопасности BLE-канала: без обязательного pairing (защита на SSH) или обязательный LE SC pairing с окном по кнопке/passkey; способ доверия ключу хоста (QR / TOFU).
3. Целевые версии ОС/BlueZ: поддерживать ли Bookworm (BlueZ 5.66) наравне с Trixie.
4. Android-клиент: форк ConnectBot или собственное приложение на его библиотеках.
5. Язык серверного демона: Rust (bluer) или Python.
6. Политика завершения сессии при разрыве: SIGHUP «как ssh» (tmux выживает) или принудительное завершение logind-сессии.
7. Нужен ли режим совместимости с NUS/сторонними UART-приложениями.
8. Мульти-клиент: требуется ли в первой версии.

Развёрнутые формулировки с рекомендациями — в сообщении оператору по завершении этапа 1.
