# Спецификация сетевых пакетов Protocol 14 и архитектуры игрока (Project Poseidon Parity)

Этот документ фиксирует полную спецификацию реализации сетевого протокола 14 (Minecraft Beta 1.7.3 / Project Poseidon / CB1060) и модель сущности игрока в `flintcore`.

---

## 1. Реестр всех 57 сетевых пакетов Protocol 14

Все числовые примитивы кодируются в формате Big-Endian. Стандартные строки кодируются как `i16 length` + UTF-16BE символы (максимальная длина проверяется на уровне декодера). Оконные заголовки контейнеров используют Java `writeUTF` (`u16 length` + UTF-8 байты).

| ID | Название | Направление | Размер / Поля полезной нагрузки | Назначение |
|:---:|---|:---:|---|---|
| **0x00** (0) | `KeepAlive` | Two-Way | 0 байт | Пинг связи между клиентом и сервером (раз в 20 тиков / 1 сек). |
| **0x01** (1) | `Login` | Two-Way | `protocol: i32, username: String, map_seed: i64, dimension: i8` | Рукопожатие авторизации и начальные параметры мира. |
| **0x02** (2) | `Handshake` | Two-Way | `username: String` | Первоначальное сетевое приветствие. |
| **0x03** (3) | `Chat` | Two-Way | `message: String` (макс 119 символов) | Чат-сообщения и системные уведомления. |
| **0x04** (4) | `UpdateTime` | Clientbound | `time: i64` | Синхронизация игрового времени (0..24000 тиков). |
| **0x05** (5) | `EntityEquipment` | Clientbound | `entity_id: i32, slot: i16, item_id: i16, damage: i16` | Отображение надетой брони (1..4) и удерживаемого предмета (0). |
| **0x06** (6) | `SpawnPosition` | Clientbound | `x: i32, y: i32, z: i32` | Координаты точки спауна мира (для компаса). |
| **0x07** (7) | `UseEntity` | Serverbound | `user: i32, target: i32, left_click: bool` | Атака или взаимодействие с сущностью. |
| **0x08** (8) | `UpdateHealth` | Clientbound | `health: i16` (0..20) | Отображение сердец здоровья игрока. |
| **0x09** (9) | `Respawn` | Two-Way | `dimension: i8` | Запрос на возрождение игрока / смена измерения. |
| **0x0A** (10) | `Flying` | Two-Way | `on_ground: bool` | Уведомление о контакте с землей без смены координат. |
| **0x0B** (11) | `PlayerPosition` | Two-Way | `x: f64, y: f64, stance: f64, z: f64, on_ground: bool` | Обновление координат игрока. |
| **0x0C** (12) | `PlayerLook` | Two-Way | `yaw: f32, pitch: f32, on_ground: bool` | Обновление угла обзора игрока. |
| **0x0D** (13) | `PlayerLookMove` | Two-Way | `x, y, stance, z, yaw, pitch, on_ground` | Полное обновление положения и вращения игрока. |
| **0x0E** (14) | `BlockDig` | Serverbound | `status: i8, x: i32, y: u8, z: i32, face: u8` | Добыча блока (0=старт, 2=завершение, 4=выброс предмета). |
| **0x0F** (15) | `Place` | Serverbound | `x: i32, y: u8, z: i32, direction: u8, item: SlotItem` | Установка блока / использование предмета (еда, ведро). |
| **0x10** (16) | `BlockItemSwitch` | Serverbound | `slot: i16` (0..8) | Выбор активного слота на хотбаре. |
| **0x11** (17) | `Sleep` | Clientbound | `entity_id: i32, in_bed: i8, bed_x: i32, bed_y: u8, bed_z: i32` | Анимация сна на кровати. |
| **0x12** (18) | `ArmAnimation` | Two-Way | `entity_id: i32, animation: u8` (1 = взмах) | Взмах рукой игрока. |
| **0x13** (19) | `EntityAction` | Serverbound | `entity_id: i32, action: u8` (1=присед, 2=встать) | Переключение состояния крадучести (sneak). |
| **0x14** (20) | `NamedEntitySpawn` | Clientbound | `id: i32, name: String, x: i32, y: i32, z: i32, yaw: u8, pitch: u8, item: i16` | Спавн другого игрока в области видимости. |
| **0x15** (21) | `PickupSpawn` | Clientbound | `id: i32, item: i16, count: i8, damage: i16, x, y, z, mot_x, mot_y, mot_z` | Спавн выпавшего предмета в мире. |
| **0x16** (22) | `Collect` | Clientbound | `collected_id: i32, collector_id: i32` | Анимация притягивания и подбора предмета. |
| **0x17** (23) | `VehicleSpawn` | Clientbound | `id: i32, type: u8, x: i32, y: i32, z: i32, thrower_id: i32, velocity: Option<(i16, i16, i16)>` | Спавн лодки, вагонетки, стрелы, TNT, снежка. |
| **0x18** (24) | `MobSpawn` | Clientbound | `id: i32, mob_type: u8, x: i32, y: i32, z: i32, yaw: u8, pitch: u8, metadata: Vec<u8>` | Спавн моба с метаданными DataWatcher. |
| **0x19** (25) | `EntityPainting` | Clientbound | `id: i32, title: String, x: i32, y: i32, z: i32, direction: i32` | Спавн картины на стене. |
| **0x1B** (27) | `Packet27` | Disabled | 18 байт | Пакет управления транспортом (отключен в CB1060). |
| **0x1C** (28) | `EntityVelocity` | Clientbound | `entity_id: i32, mot_x: i16, mot_y: i16, mot_z: i16` | Импульс скорости сущности (`* 8000.0`). |
| **0x1D** (29) | `DestroyEntity` | Clientbound | `entity_id: i32` | Удаление сущности из клиента. |
| **0x1E** (30) | `Entity` | Clientbound | `entity_id: i32` | Базовый пакет обновления сущности без смещения. |
| **0x1F** (31) | `RelEntityMove` | Clientbound | `entity_id: i32, dx: i8, dy: i8, dz: i8` | Относительное смещение ($|\Delta| < 4$ блоков, в $1/32$). |
| **0x20** (32) | `EntityLook` | Clientbound | `entity_id: i32, yaw: u8, pitch: u8` | Поворот сущности без перемещения. |
| **0x21** (33) | `RelEntityMoveLook`| Clientbound | `entity_id: i32, dx, dy, dz, yaw, pitch` | Совмещенное смещение и поворот сущности. |
| **0x22** (34) | `EntityTeleport` | Clientbound | `entity_id: i32, x: i32, y: i32, z: i32, yaw: u8, pitch: u8` | Абсолютная телепортация сущности (`* 32`). |
| **0x26** (38) | `EntityStatus` | Clientbound | `entity_id: i32, status: u8` (2=урон, 3=смерть) | Статусные анимации и события сущности. |
| **0x27** (39) | `AttachEntity` | Clientbound | `entity_id: i32, vehicle_id: i32` (-1 = спешиться) | Посадка игрока в вагонетку/лодку. |
| **0x28** (40) | `EntityMetadata` | Clientbound | `entity_id: i32, metadata: Vec<u8>` | Обновление флагов DataWatcher (горение, присед). |
| **0x32** (50) | `PreChunk` | Clientbound | `x: i32, z: i32, mode: bool` | Инициализация / выгрузка сетки чанка в клиенте. |
| **0x33** (51) | `MapChunk` | Clientbound | 17 байт заголовок + 81920 байт сжатых zlib блоков/света | Передача блоков и освещения столбца чанка. |
| **0x34** (52) | `MultiBlockChange`| Clientbound | `cx: i32, cz: i32, count: u16, coords, types, metas` | Пакетное обновление до сотен блоков в одном чанке. |
| **0x35** (53) | `BlockChange` | Clientbound | `x: i32, y: u8, z: i32, block_type: u8, metadata: u8` | Одиночное изменение блока в мире. |
| **0x36** (54) | `PlayNoteBlock` | Clientbound | `x: i32, y: i16, z: i32, instrument: u8, pitch: u8` | Воспроизведение звука нотного блока / поршня. |
| **0x3C** (60) | `Explosion` | Clientbound | `x: f64, y: f64, z: f64, radius: f32, records: Vec<(i8,i8,i8)>` | Взрыв TNT / крипера со списком разрушенных блоков. |
| **0x3D** (61) | `LevelSound` | Clientbound | `effect_id: i32, x: i32, y: u8, z: i32, data: i32` | Частицы и звуки мира (2001 = разбитие блока). |
| **0x46** (70) | `Bed` | Clientbound | `reason: u8` (0=кровать занята/невалидна, 1=дождь, 2=ясно) | Уведомление о состоянии сна и погоды. |
| **0x47** (71) | `Weather` | Clientbound | `entity_id: i32, type: u8 (1), x: i32, y: i32, z: i32` | Спавн удара молнии. |
| **0x64** (100)| `OpenWindow` | Clientbound | `id: u8, type: u8, title: String (UTF-8), slots: u8` | Открытие окна верстака, сундука, печи, раздатчика. |
| **0x65** (101)| `CloseWindow` | Two-Way | `window_id: u8` | Закрытие контейнера клиентом или сервером. |
| **0x66** (102)| `WindowClick` | Serverbound | `id: u8, slot: i16, button: u8, action: i16, shift: bool, item: SlotItem` | Клик по предмету в инвентаре/окне. |
| **0x67** (103)| `SetSlot` | Clientbound | `window_id: i8, slot: i16, item: Option<SlotItem>` | Синхронизация одиночного слота (окно -1 = курсор). |
| **0x68** (104)| `WindowItems` | Clientbound | `window_id: u8, count: i16, items: Vec<Option<SlotItem>>` | Полная синхронизация содержимого открытого окна. |
| **0x69** (105)| `CraftProgressBar`| Clientbound | `window_id: u8, property: i16, value: i16` | Индикаторы плавки и горения топлива в печи. |
| **0x6A** (106)| `Transaction` | Two-Way | `window_id: u8, action_number: i16, accepted: bool` | Подтверждение валидности клика транзакции инвентаря. |
| **0x82** (130)| `UpdateSign` | Two-Way | `x: i32, y: i16, z: i32, lines: [String; 4]` | 4 строки текста на табличке. |
| **0x83** (131)| `MapData` | Clientbound | `item_type: i16, item_id: i16, data: Vec<u8>` | Данные пикселей и маркеров карты в руке. |
| **0xC8** (200)| `Statistic` | Clientbound | `statistic_id: i32, amount: u8` | Инкремент игровой статистики или ачивки. |
| **0xFF** (255)| `KickDisconnect` | Two-Way | `reason: String` | Отключение соединения с указанием причины. |

---

## 2. Модель игрока (`src/entity/player.rs` и `src/gameplay/inventory.rs`)

### 2.1. Инвентарь игрока (`InventoryPlayer`)
- **Основной массив предметов**: `items: [Option<ItemStack>; 36]`:
  - Слоты `0..=8`: хотбар. Активный слот определяется `held_slot: usize` (0..=8).
  - Слоты `9..=35`: сетка рюкзака (3 ряда по 9 слотов).
- **Слоты брони**: `armor: [Option<ItemStack>; 4]`:
  - `armor[0]`: Ботинки.
  - `armor[1]`: Поножи.
  - `armor[2]`: Нагрудник.
  - `armor[3]`: Шлем.
- **Курсор мыши**: `cursor: Option<ItemStack>` — предмет, перемещаемый курсором между слотами при открытом GUI.
- **Сетка крафта**: `craft_grid: [Option<ItemStack>; 4]` (2x2), `craft_result: Option<ItemStack>`.

### 2.2. Оконная адресация контейнера игрока (`ContainerPlayer`, окно ID 0)
- Слот `0`: Результат крафта (`craft_result`).
- Слоты `1..=4`: 2x2 матрица крафта (`craft_grid`).
- Слоты `5..=8`: Слоты брони (`armor[3]` шлем, `armor[2]` нагрудник, `armor[1]` поножи, `armor[0]` ботинки).
- Слоты `9..=35`: Основной инвентарь (`items[9..=35]`).
- Слоты `36..=44`: Хотбар (`items[0..=8]`).
- Слот `-1`: Курсор мыши.

### 2.3. Алгоритм снижения урона броней (Poseidon Parity)
- Сумма очков брони `total_points = sum(piece.armor_defense())`.
- Коэффициент поглощения: $\text{factor} = 25 - \text{total\_points}$.
- Итоговый урон: $\text{actual\_damage} = \frac{\text{raw\_damage} \times \text{factor} + 12}{25}$.
- Износ прочности брони распределяется равномерно на все надетые элементы.

---

## 3. Движок отслеживания сущностей (`src/entity/tracker.rs`)

Реализует 100% поведение `EntityTracker.java` и `EntityTrackerEntry.java`:

### 3.1. Периодичность и квантование
- Запуск тика трекера: каждые 2 тика симуляции (`tick_count % 2 == 0`).
- Квантование координат: $\text{encoded} = \lfloor \text{coord} \times 32.0 \rfloor$.
- Квантование углов (`pack_angle`): $\text{scaled} = \lfloor \text{angle} \times 256.0 / 360.0 \rfloor$, $\text{packed} = (\text{scaled as i8}) \text{ as u8}$. Отрицательные углы (поворот влево и взгляд вверх) циклически проецируются в полный диапазон $0..255$, исключая насыщение в 0.
- Радиус отслеживания: $\text{view\_distance} \times 16 - 16$ блоков (144 блока по умолчанию).
- Знаковое вычисление дельты вращения: `(new_yaw.wrapping_sub(last_yaw)) as i8`, исключающее артефакты при переходе границы $0^\circ / 360^\circ$.

### 3.2. Алгоритм выбора пакета передвижения
1. Рассчитываются дельты: $\Delta X = \text{newX} - \text{lastX}$, $\Delta Y = \text{newY} - \text{lastY}$, $\Delta Z = \text{newZ} - \text{lastZ}$, знаковые $\Delta \text{Yaw}$, $\Delta \text{Pitch}$.
2. Если дельты входят в диапазон $[-128, 127]$ (в пределах 4 блоков) и счетчик тиков $t < 400$:
   - Если изменились и позиция, и поворот $\to$ отправляется `Packet33RelEntityMoveLook`.
   - Если изменилась только позиция $\to$ отправляется `Packet31RelEntityMove`.
   - Если изменился только поворот (при $|\Delta \text{Yaw}| \ge 4$ или $|\Delta \text{Pitch}| \ge 4$) $\to$ отправляется `Packet32EntityLook`.
3. Если смещение превысило 4 блока или счетчик тиков $t \ge 400$ (40 секунд без абсолютной синхронизации):
   - Счетчик $t$ сбрасывается в 0.
   - Отправляется `Packet34EntityTeleport` для полного сброса погрешности с плавающей точкой.

### 3.3. Синхронизация скорости и экипировки
- **Импульсы скорости**: при установке `velocity_changed = true` (урон, отдача, взрыв) рассылается `Packet28EntityVelocity` наблюдателям и самому игроку, после чего флаг сбрасывается.
- **Экипировка**: при изменении содержимого слотов 0..=4 рассылается `Packet5EntityEquipment` всем наблюдателям.

---

## 4. Жизненный цикл игрока и боевая система (`src/server.rs`)

1. **Авторизация и вход**:
   - `Handshake` (2) $\to$ `Handshake` ("-").
   - `Login` (1) $\to$ проверка версии 14.
   - Регистрация в `EntityStore` и `EntityTracker`.
   - Отправка `Login`, `SpawnPosition`, `UpdateTime`, спаун-чанков (9x `PreChunk` + `MapChunk`), `PlayerLookMove`.
   - Начальная синхронизация инвентаря `WindowItems` (45 слотов) и здоровья `UpdateHealth(20)`.
   - Рассылка сообщения о входе `join_message` другим игрокам.
2. **Взаимодействие с миром**:
   - **Копка (`BlockDig`)**: проверка дальности ($\le 36.0$), спаун-защита, расчет повреждения инструмента, эффект `LevelSound(2001)`, `BlockChange` (воздух).
   - **Установка (`Place`)**: открытие интерфейса контейнеров (верстак 58 $\to$ `OpenWindow(type=1, slots=9)`), дедупликация ПКМ, употребление еды при клике по воздуху (`direction == 255`), расчет координат примыкания, спаун-защита, установка блока, `BlockChange`, списание предмета из хотбара, синхронизация слота `SetSlot`.
   - **Оконные интерфейсы и клики (`WindowClick` / `CloseWindow`)**:
     - При клике вне окна (`slot == -999`): предмет сбрасывается с курсора, синхронизируется только курсор (`SetSlot(-1, -1)`).
     - При закрытии окна клиентом `CloseWindow`: сервер освобождает курсор в инвентарь без отправки эхо-пакета `CloseWindow` (исключая зацикливание закрытия экранов).
3. **Единый конвейер движения и урон от падения**:
   - Обработка всех пакетов движения (`Flying`, `PlayerPosition`, `PlayerLook`, `PlayerLookMove`) объединена в `handle_movement`.
   - При снижении (`y < prev_y && !on_ground`) аккумулируется `fall_distance`.
   - При контакте с землей (`on_ground == true`): если `fall_distance > 3.0`, наносится чистый урон `ceil(fall_distance - 3.0)` без редукции броней (`take_damage_direct`). `fall_distance` сбрасывается в 0.
4. **Боевая система (PvP в `Packet7UseEntity`)**:
   - Атака цели (`attack == true`): проверка досягаемости ($d^2 \le 16.0$) и таймера неуязвимости (`target.no_damage_ticks <= 10`).
   - Урон оружия: алмазный меч = 7, железный = 6, каменный = 5, деревянный/золотой = 4, кулак = 1.
   - Снижение броней: $\text{factor} = 25 - \text{armor}$, $\text{damage} = \max(1, (\text{raw} \times \text{factor} + 12) / 25)$.
   - Импульс отдачи (Knockback): вектор от атакующего к цели, скорость `mot_x += dx * 0.4`, `mot_y = 0.4`, `mot_z += dz * 0.4`. Отправка `EntityVelocity` цели и наблюдателям.
   - Взмах рукой атакующего (`ArmAnimation`), анимация урона цели (`EntityStatus(2)`), износ оружия атакующего и брони цели.
   - Смерть: `EntityStatus(3)` и сообщение в чат `§e{victim} was slain by {attacker}`.
5. **Чат и поддержка цветов (`§` и `&`)**:
   - Символ параграфа `§` (`\u{00A7}`) полностью разрешен во входящих сообщениях.
   - Трансляция кодов `&a`..`&f`, `&0`..`&9` в `§` для удобного форматирования.
   - Алгоритм `wrap_text` поглощает символ кода цвета, исключая удвоение символов (`§e` вместо `§ee`).
6. **Стриминг чанков и кэширование сжатия**:
   - Транзакционная отправка `PreChunk` + `MapChunk`: чанк помечается в `sent_chunks` только при успешном попадании обоих пакетов в сокет/очередь, исключая потерю чанков при сетевых спайках.
   - Кэширование сжатых пакетов в `World::packet_cache` на уровне памяти для мгновенной отдачи повторным игрокам.
7. **Тик сущностей**:
   - Ежетактный декремент таймеров неуязвимости `no_damage_ticks` и горения `fire_ticks`.
8. **Отключение**:
   - Отправка `DestroyEntity` наблюдателям через `tracker.untrack_player()`.
   - Рассылка `leave_message` в чат.
   - Очистка из `EntityStore` и соединений.
