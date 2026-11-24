### Aria UI strings: Russian.
### Translation of locales/en/main.ftl: keep the same message ids and placeables.

## Navigation: sidebar entries, page headings and history buttons

nav-home = Главная
nav-search = Поиск
nav-library = Моя медиатека
nav-liked = Любимые треки
nav-playlists = Плейлисты
nav-albums = Альбомы
nav-artists = Исполнители
nav-local = Музыка на компьютере
nav-settings = Настройки
nav-back = Назад
nav-forward = Вперёд

## Empty states

# $phase (Number) - the roadmap phase that delivers the page, e.g. 2
empty-phase-pending = Будет доступно на этапе { $phase }
empty-nothing-playing = Сейчас ничего не играет
empty-right-panel = Очередь пуста

## Player controls (tooltips)

player-play = Воспроизвести
player-previous = Предыдущий трек
player-next = Следующий трек
player-volume = Громкость
player-devices = Устройства
player-queue = Очередь
player-lyrics = Текст песни
player-pause = Пауза
player-shuffle = Перемешать
player-repeat-off = Повтор: выкл.
player-repeat-all = Повтор: все треки
player-repeat-one = Повтор: один трек
player-mute = Выключить звук
player-unmute = Включить звук
player-seek = Перемотка
player-opened-no-heart = Открыт не из вашей библиотеки, поэтому без сердечка. Добавьте его папку в Настройках, раздел «Библиотека», чтобы сохранить трек.
drop-hint = Отпустите, чтобы воспроизвести. Папки добавятся в библиотеку.
player-sleep = Таймер сна
player-sleep-15 = 15 минут
player-sleep-30 = 30 минут
player-sleep-45 = 45 минут
player-sleep-60 = 1 час
player-sleep-90 = 1 час 30 минут
player-sleep-end-of-track = После этого трека
player-sleep-off = Выключить
# $minutes (Number) - whole minutes until the sleep timer pauses the music, rounded up
player-sleep-left = { $minutes ->
    [one] осталась { $minutes } мин
   *[other] осталось { $minutes } мин
}
sleep-timer-done = Таймер сна поставил музыку на паузу.
player-unknown-title = Неизвестное название
# $phase (Number) - the roadmap phase that delivers the control, e.g. 2
player-coming-in = Появится на этапе { $phase }

## Queue panel

queue-now-playing = Сейчас играет
queue-next-user = Далее в очереди
queue-next-list = Далее из списка
queue-clear = Очистить очередь
# $title - the track's title
queue-remove = Убрать из очереди: { $title }

## Settings page

settings-general = Общие
settings-language = Язык
settings-language-system = Как в системе
settings-theme = Тема
settings-theme-dark = Тёмная
settings-theme-light = Светлая
settings-theme-system = Как в системе
settings-accent-dark = Акцентный цвет тёмной темы
settings-accent-light = Акцентный цвет светлой темы
settings-accent-reset = Сбросить
settings-minimize-to-tray = Сворачивать в трей
settings-minimize-to-tray-hint = Окно будет сворачиваться в системный трей, а не на панель задач. Заработает на этапе 4.
settings-playback = Воспроизведение
settings-output-device = Устройство вывода
settings-device-system-default = Системное по умолчанию
settings-device-unavailable = Недоступное устройство
settings-gapless = Непрерывное воспроизведение
settings-watch-folders = Следить за изменениями в папках
settings-watch-folders-hint = Музыка, добавленная, удалённая или изменённая в этих папках, появится сама через пару секунд после окончания копирования. Некоторые сетевые папки не сообщают об изменениях: для них есть «Пересканировать».
settings-replay-gain = Выравнивание громкости (ReplayGain)
settings-replay-gain-off = Выкл.
settings-replay-gain-track = По трекам
settings-replay-gain-album = По альбомам
settings-replay-gain-hint = Треки звучат одинаково громко по тегам ReplayGain в файлах и никогда не громче, чем можно без искажений. Файлы без этих тегов играют как есть. Действует со следующего трека; выровненный трек уже не bit-perfect.
settings-back-up = Резервная копия…
settings-restore = Восстановить…
settings-backup-hint = В копии — ваши папки, теги треков, избранное, плейлисты, история, звуки и настройки, но не музыка: файлы остаются на своих местах. Восстановление происходит при следующем запуске Aria, а текущая библиотека сохраняется рядом.
# $name (String) - the backup folder's name, e.g. Aria backup 2026-11-20 14.05
backup-done = Резервная копия сохранена в папку { $name }.
backup-failed = Не удалось сделать резервную копию.
restore-staged = Копия готова. Закройте Aria и запустите снова, чтобы восстановить её.
restore-not-a-backup = В этой папке нет резервной копии Aria. Выберите папку «Aria backup» с датой.
restore-newer = Эту копию сделала более новая версия Aria, и эта версия не может её восстановить.
restore-failed = Не удалось восстановить резервную копию.
restore-done = Восстановлено из резервной копии. Прежняя библиотека сохранена рядом под именем «library.db.before-restore».
playlist-import = Импорт…
playlist-export = Экспорт…
# $name (String) - the playlist's name; $count (Number) - how many entries it got
playlist-imported = { $count ->
    [one] Создан плейлист { $name }: { $count } запись.
    [few] Создан плейлист { $name }: { $count } записи.
   *[other] Создан плейлист { $name }: { $count } записей.
}
playlist-import-empty = В этом файле плейлиста нет ни одного файла.
# $name (String) - the playlist's name
playlist-exported = Плейлист { $name } сохранён.
playlist-file-failed = Не удалось прочитать или записать файл плейлиста.
settings-updates = Обновления
settings-check-updates = Проверить обновления
settings-check-updates-hint = Открывает в браузере страницу последнего релиза. Сама Aria ничего не отправляет в сеть.
settings-history = Вести историю прослушиваний
settings-history-hint = История хранится только на этом компьютере.
settings-library = Библиотека
settings-folders-empty = Папок пока нет. Добавьте папку с вашей музыкой.
settings-folder-unavailable = Недоступна
settings-remove = Удалить
settings-remove-named = Удалить { $folder }
settings-remove-folder-title = Убрать эту папку?
settings-remove-folder-body = Папка будет убрана из библиотеки. Её файлы останутся на диске.
settings-clear-artwork = Очистить кэш обложек
settings-clear-artwork-title = Очистить кэш обложек?
settings-clear-artwork-body = Сохранённые обложки будут удалены. Они создадутся заново при следующем сканировании библиотеки.
settings-history-clear = Очистить историю прослушиваний
settings-history-clear-confirm = Очистить историю прослушиваний?
settings-history-clear-body = Список «Недавно играло» будет очищен. Избранное и плейлисты останутся. Это нельзя отменить.
settings-advanced = Дополнительно
settings-log-level = Уровень журналирования
settings-log-level-error = Ошибки
settings-log-level-warn = Предупреждения
settings-log-level-info = Информация
settings-log-level-debug = Отладка
settings-log-level-trace = Трассировка
settings-open-config = Открыть папку настроек
settings-open-logs = Открыть папку журналов
settings-reset = Сбросить настройки
settings-reset-confirm-title = Сбросить все настройки?
settings-reset-confirm-body = Все настройки вернутся к значениям по умолчанию. Это действие нельзя отменить.
settings-cancel = Отмена
settings-confirm = Подтвердить
settings-about = О программе
settings-version = Версия
settings-license = Лицензия
settings-third-party = Лицензии сторонних компонентов перечислены в файле THIRD_PARTY_NOTICES.md

## Notices (toasts)

notice-settings-recovered = Не удалось прочитать файл настроек, поэтому восстановлены значения по умолчанию. Резервная копия старого файла сохранена.
notice-settings-readonly = Нет доступа к файлу настроек, поэтому используются значения по умолчанию. Изменения могут не сохраниться.
notice-settings-save-failed = Не удалось сохранить настройки. Aria попробует снова при следующем изменении.
notice-open-folder-failed = Не удалось открыть папку.
notice-pick-folder-failed = Не удалось открыть выбор папки.
notice-pick-sounds-failed = Не удалось открыть выбор файлов.
notice-dismiss = Закрыть

## Library notices (toasts)

# $count (Number) - how many files and folders the scan could not read
library-scan-skipped = { $count ->
    [one] Не удалось прочитать { $count } файл или папку, при сканировании они пропущены.
    [few] Не удалось прочитать { $count } файла или папки, при сканировании они пропущены.
   *[other] Не удалось прочитать { $count } файлов или папок, при сканировании они пропущены.
}
# $count (Number) - how many of the files opened were skipped
open-skipped = { $count ->
    [one] Пропущен { $count } файл: Aria не играет такой формат или не смогла его прочитать.
    [few] Пропущено { $count } файла: Aria не играет такой формат или не смогла их прочитать.
   *[other] Пропущено { $count } файлов: Aria не играет такой формат или не смогла их прочитать.
}
# $folder (String) - the last part of the folder's path, e.g. Music
library-folder-unavailable = Папка { $folder } недоступна, или все её файлы перемещены либо удалены. Треки сохранены, а папку можно удалить в настройках.
library-folder-nested = Эта папка находится внутри папки, которая уже есть в библиотеке.
library-folder-duplicate = Эта папка уже есть в библиотеке.
library-recovered = База данных библиотеки была повреждена, поэтому создана новая. Старый файл сохранён как резервная копия.
library-newer = Библиотека сохранена более новой версией Aria, поэтому она отключена. Обновите Aria, чтобы пользоваться ею.
library-unavailable = Не удалось открыть библиотеку, поэтому она пока отключена. Остальные функции Aria продолжают работать.
library-action-failed = В библиотеке произошла ошибка, и действие не завершено. Ваши музыкальные файлы не изменены.
playlist-name-invalid = Название плейлиста должно быть от 1 до 100 символов.
track-favorite = Добавить в избранное
track-unfavorite = Убрать из избранного
track-add-to-playlist = Добавить в плейлист
playlist-new = Новый плейлист…
playlist-name-label = Название
playlist-create = Создать
playlist-rename = Переименовать

## Playback notices (toasts)

# $title (String) - the track's title, e.g. Blue in Green
playback-track-failed = Не удалось воспроизвести «{ $title }», трек пропущен.
playback-stopped-after-failures = Воспроизведение остановлено: несколько треков подряд воспроизвести не удалось.
playback-device-fallback = Аудиоустройство недоступно, поэтому Aria переключилась на другое.
playback-no-device = Нет доступного аудиоустройства, поэтому воспроизведение приостановлено.
exclusive-unavailable = Монопольный режим сейчас недоступен на этом устройстве, поэтому Aria использует системный микшер.

## Soundboard

sound-name-invalid = Название звука должно быть от 1 до 50 символов.
sound-limit = Достигнут предел звуков ({ $max }), поэтому часть файлов не добавлена.
sound-missing = Файл этого звука не найден, поэтому ничего не воспроизведено.
sound-failed = Не удалось воспроизвести этот звук. Музыка не затронута.
sound-device-failed = Не удалось использовать второе устройство для звуков.
microphone-failed = Не удалось открыть микрофон. Музыка и звуки играют дальше.
microphone-denied = Система не дала Aria доступ к микрофону. Разрешите доступ к микрофону для классических приложений в настройках конфиденциальности.
microphone-on = Микрофон включён
microphone-off = Микрофон выключен
microphone-needs-device = Чтобы передавать микрофон в чат, выберите второе устройство в настройках, раздел «Звуковая панель».
microphone-is-cable = Выбранный микрофон — виртуальный кабель, поэтому он не открыт: чат слышал бы сам себя. Выберите настоящий микрофон в настройках, раздел «Звуковая панель».
hotkey-not-allowed = Сочетание клавиш должно включать Ctrl, Alt или Win, если это не клавиша F или клавиша цифрового блока.
hotkey-taken = Это сочетание клавиш уже используется в Aria.
hotkey-unavailable = Система отклонила это сочетание клавиш — возможно, его использует другая программа. Сочетание не сохранено.
# $count (Number) - how many hotkeys the system refused to register
hotkeys-inactive = { $count ->
    [one] { $count } сочетание клавиш не работает: система его отклонила — возможно, его использует другая программа.
    [few] { $count } сочетания клавиш не работают: система их отклонила — возможно, их используют другие программы.
    [many] { $count } сочетаний клавиш не работают: система их отклонила — возможно, их используют другие программы.
   *[other] { $count } сочетания клавиш не работают: система их отклонила — возможно, их используют другие программы.
}
hotkey-inactive-hint = Система отклонила это сочетание клавиш — возможно, его использует другая программа. Сейчас оно не работает.
hotkey-dialog-title = Задать сочетание клавиш
hotkey-dialog-hint = Нажмите клавиши одновременно. Escape — отмена, Delete или Backspace — убрать сочетание.
hotkey-dialog-clear = Убрать
hotkeys-unavailable = Глобальные сочетания клавиш недоступны в этой системе, поэтому ни одно из них не работает.
hotkey-dialog-numpad = Клавиша цифрового блока

## Local Music page

local-add-folder = Добавить папку
local-rescan = Пересканировать
local-tab-tracks = Треки
local-tab-albums = Альбомы
local-tab-artists = Исполнители
local-search = Поиск по музыке
local-col-number = #
local-col-title = Название
local-col-artist = Исполнитель
local-col-album = Альбом
local-col-duration = Длительность
local-play = Воспроизвести
local-play-next = Играть следующим
local-add-to-queue = Добавить в очередь
local-back = Все треки
local-empty-title = Музыки пока нет
local-empty-body = Добавьте папку с музыкой, и Aria найдёт её.
local-no-tracks-title = Треки не найдены
local-no-tracks-body = Треки появятся здесь, когда Aria прочитает ваши папки.
local-no-results = По вашему запросу ничего не найдено.
# $count (Number) - how many tracks an album or an artist has
local-track-count = { $count ->
    [one] { $count } трек
    [few] { $count } трека
    [many] { $count } треков
   *[other] { $count } трека
}
# $count (Number) - how many audio files the running scan has looked at so far
library-scanning = Идёт сканирование папок, просмотрено файлов: { $count }
library-unknown-artist = Неизвестный исполнитель
library-unknown-album = Неизвестный альбом

## Fatal startup errors (a message box shown before Aria quits)

error-startup-title = Не удалось запустить Aria
error-startup-window = Aria не удалось открыть окно.
instance-unreachable-title = Aria уже открыта
instance-unreachable-body = Другое окно Aria уже открыто, но оно не ответило. Переключитесь на него или закройте его (через диспетчер задач, если оно не отвечает) и запустите Aria снова.

## Language names: endonyms, identical in every locale

lang-en = English
lang-ru = Русский
lang-hy = Հայերեն

# Home page
home-recent = Недавно играло
home-added = Недавно добавлено
home-empty = Библиотека пуста
home-empty-hint = Добавьте папку с музыкой в разделе «Настройки», «Библиотека».
home-recent-empty = Пока ничего не играло.

# Liked Songs page
liked-empty = Пока нет избранного
liked-empty-hint = Нажмите на сердечко рядом с треком.

# Playlists page
playlists-empty = Плейлистов пока нет
playlists-empty-hint = Создайте плейлист здесь или выберите «Добавить в плейлист» в меню трека.
playlist-empty = Этот плейлист пуст
playlist-delete = Удалить плейлист
# $name (String) - the playlist's name
playlist-delete-confirm = Удалить плейлист «{ $name }»? Его треки останутся в вашей библиотеке.
playlist-remove-entry = Убрать из плейлиста
# $count (Number) - how many entries of the playlist can't be shown because their files are gone or their folder was removed
playlist-unavailable = { $count ->
    [one] { $count } трек недоступен
    [few] { $count } трека недоступны
    [many] { $count } треков недоступны
   *[other] { $count } трека недоступны
}
# $count (Number) - how many tracks a playlist has
playlist-track-count = { $count ->
    [one] { $count } трек
    [few] { $count } трека
    [many] { $count } треков
   *[other] { $count } трека
}

## Output path, exclusive mode and quality
output-bit-perfect = Без изменений
output-bit-perfect-hint = Aria передаёт отсчёты трека устройству без изменений. Запуск, пауза, возобновление и перемотка идут с затуханием за 5 мс; на стыках без пауз между треками затухания нет.
output-resampled = С передискретизацией
output-resampled-hint = Монопольный вывод на частоте, которой нет у трека, поэтому отсчёты пересчитываются.
output-converted = С преобразованием
output-converted-hint = Монопольный вывод, но отсчёты не совпадают с файлом: источник с потерями или с плавающей точкой, разрядность неизвестна, больше 24 бит или уменьшена, изменились каналы или звук выключен.
output-mixed = Смешано со звуком
output-mixed-hint = В поток подмешан звук из саундборда, поэтому вывод не бит-в-бит.
output-system-mixer = Системный микшер
output-system-mixer-hint = Воспроизведение через системный микшер, который может менять отсчёты.
player-volume-locked = Громкость задаётся на устройстве, пока включён монопольный режим.
player-mic-on = Микрофон включён: ваш голос идёт на второе устройство. Нажмите, чтобы выключить.
player-mic-failed = Не удалось открыть микрофон. Нажмите, чтобы выключить его.
player-mic-device = Микрофон: { $name }
settings-exclusive = Монопольный режим
settings-exclusive-hint = Пока играет Aria, другие программы молчат на этом устройстве. Уровень задайте на самом устройстве.
settings-exclusive-windows-only = Монопольный режим доступен только в Windows.
quality-lossless = Без потерь
quality-hi-res = Hi-Res
local-col-quality = Качество
local-lossless-only = Только без потерь

## Soundboard page and settings

nav-soundboard = Звуковая панель
soundboard-empty = Звуков пока нет
soundboard-empty-hint = Добавьте короткие аудиофайлы, чтобы запускать их щелчком или сочетанием клавиш.
soundboard-add = Добавить звуки…
soundboard-stop-all = Остановить все
sound-rename = Переименовать
sound-hotkey = Сочетание клавиш…
sound-hotkey-none = Нет
sound-volume = Громкость
sound-remove = Убрать
# $name (String) - the sound's name
sound-remove-confirm = Убрать «{ $name }» из звуковой панели? Файл не удаляется.
settings-soundboard = Звуковая панель
settings-sound-device = Второе устройство
settings-sound-device-none = Нет
settings-sound-target = Воспроизводить звуки на
settings-sound-target-mine = Моём устройстве
settings-sound-target-second = Втором устройстве
settings-sound-target-both = Обоих устройствах
settings-sound-target-note = Второе устройство не выбрано, поэтому звуки воспроизводятся на вашем устройстве.
settings-global-hotkeys = Глобальные сочетания клавиш
settings-global-hotkeys-hint = Сочетания работают, когда фокус у другой программы. Aria получает только заданные вами сочетания.
settings-stop-hotkey = Сочетание для «Остановить все»
settings-voice-chat-hint = Aria не устанавливает виртуальный кабель и никогда не воспроизводит звук прямо в настоящий микрофон. Если кабель выбран вторым устройством и микрофоном в чате, чат слышит звуки, общую музыку и ваш микрофон.
settings-share-music = Музыка и на второе устройство
settings-share-music-hint = Друзья в чате слышат то, что вы включаете. Ваша громкость и отключение звука на это не влияют.
settings-share-music-volume = Громкость музыки в чате
settings-mic-device = Микрофон
settings-mic-is-cable = Это виртуальный кабель, а не микрофон: если передавать его в кабель, чат будет слышать сам себя, поэтому Aria его не откроет. Выберите настоящий микрофон.
settings-mic-default = Системный по умолчанию
settings-mic-send = Передавать мой микрофон на второе устройство
settings-mic-send-hint = Микрофон открыт, только пока это включено и выбрано второе устройство. Его звук идёт только туда; Aria его не записывает и не хранит.
settings-mic-volume = Громкость микрофона
settings-mic-hotkey = Сочетание для включения и выключения микрофона
settings-chat-needs-device = Выберите второе устройство, чтобы передавать в чат музыку или микрофон.
settings-chat-same-device = Второе устройство то же, на котором играет музыка, поэтому в чат ничего не попадёт. Выберите виртуальный кабель.
# $name (String) - the name of the virtual cable found
settings-cable-found = Найден виртуальный кабель: { $name }
settings-cable-use = Выбрать
settings-cable-guide-1 = Чтобы передавать звук в чат, установите бесплатный виртуальный кабель, например VB-CABLE:
settings-cable-guide-link = VB-CABLE, официальная страница
settings-cable-guide-2 = Затем перезапустите Aria или снова откройте настройки.
settings-cable-guide-3 = Выберите кабель здесь вторым устройством. В программе чата выберите микрофоном его сторону записи (у VB-CABLE это «CABLE Output»).
settings-cable-hint-vb = В программе чата выберите микрофоном «CABLE Output».
settings-cable-hint-other = В программе чата выберите микрофоном устройство записи этого кабеля.

## Spotify notices
spotify-session-ended = Сеанс Spotify завершён. Подключитесь снова в настройках.
spotify-port-busy = Aria не может принять вход: порт 43819 на этом компьютере занят.
spotify-sign-in-denied = Вход в Spotify отменён.
spotify-sign-in-timed-out = Вход в Spotify занял слишком много времени. Попробуйте ещё раз.
spotify-browser-failed = Aria не смогла открыть браузер для входа в Spotify.
spotify-offline = Aria не может связаться со Spotify. Проверьте подключение.
# $seconds (Number) - how long Spotify asked Aria to wait
spotify-rate-limited = Spotify попросил Aria подождать { $seconds } с, прежде чем снова обращаться.
spotify-premium-required = Для воспроизведения на устройствах Spotify нужен Spotify Premium.
spotify-no-device = Нет активного устройства Spotify. Сначала откройте Spotify на каком-нибудь устройстве.
spotify-not-allow-listed = Этого аккаунта Spotify ещё нет в списке доступа Aria. Попросите владельца приложения добавить его в панели разработчика Spotify.
# $message (String) - Spotify's own error message, in English
spotify-error = Ответ Spotify: { $message }
spotify-keyring-failed = Aria не смогла сохранить вход в Spotify; в следующий раз нужно будет подключиться снова.
spotify-signed-in-page = Вы подключены к Aria. Эту вкладку можно закрыть.

## Settings: Spotify
settings-spotify = Spotify
settings-spotify-connect = Подключить Spotify
settings-spotify-waiting = Ожидание браузера…
settings-spotify-cancel = Отмена
# $name (String) - the Spotify account's display name
settings-spotify-connected = Подключено: { $name }
settings-spotify-device = Воспроизводить на
settings-spotify-active-device = Активное устройство
settings-spotify-refresh = Обновить медиатеку
settings-spotify-disconnect = Отключить
settings-spotify-disconnect-confirm = Отключить Spotify? Aria забудет вход и вашу медиатеку Spotify.
settings-spotify-allow-list = Пока приложение Aria для Spotify в режиме разработки, подключиться могут только аккаунты, которые добавил его владелец (до 5).
settings-spotify-no-keyring = В этой системе нет хранилища ключей, поэтому подключаться придётся при каждом запуске Aria.
settings-spotify-premium = Для воспроизведения на ваших устройствах нужен Spotify Premium.

## Player bar: Spotify
# $device (String) - the name of the Spotify device playing
player-spotify-on = На устройстве { $device }
player-spotify-not-here = Недоступно для воспроизведения Spotify
player-devices-connect = Подключите Spotify в настройках, чтобы выбрать устройство

## Spotify pages
spotify-page-signed-out = Подключите Spotify, чтобы видеть здесь свою музыку.
spotify-loading = Загрузка медиатеки Spotify…
spotify-search-hint = Поиск в Spotify
spotify-search-none = В Spotify ничего не найдено.
spotify-songs = Песни
spotify-albums = Альбомы
spotify-artists = Исполнители
spotify-playlists = Плейлисты
# $count (Number) - how many saved tracks there are
spotify-liked = Любимые треки ({ $count })
spotify-empty = Здесь пока ничего нет.
# $owner (String) - the playlist owner's name
spotify-by = автор: { $owner }

## Spotify: review fixes
spotify-load-failed = Aria не смогла загрузить это из Spotify. Попробуйте «Обновить медиатеку» в настройках.
spotify-sign-in-failed-page = Aria не подключена к Spotify. Вернитесь в Aria и попробуйте ещё раз.

## Full-screen record
player-full-screen = На весь экран
player-exit-full-screen = Выйти из полноэкранного режима
vinyl-finish = Вид пластинки
vinyl-finish-random = Случайный
vinyl-finish-black = Чёрная
vinyl-finish-tinted = Тонированная
vinyl-finish-splatter = С брызгами
vinyl-speed = Скорость
vinyl-rpm = { $rpm } об/мин
player-mini-player = Мини-плеер
mini-title = Мини-плеер Aria
mini-back = Вернуться в Aria
mini-on-top = Поверх всех окон
mini-close = Закрыть мини-плеер
vinyl-finish-marble = Мраморная
settings-spotify-play-in-aria = Играть в Aria
settings-spotify-play-in-aria-hint = Spotify играет в маленьком окне Edge или Chrome, которое Aria открывает и закрывает сама. Его можно свернуть. Нужен Spotify Premium.
settings-spotify-reconnect = Переподключить
settings-spotify-reconnect-hint = Переподключите один раз, чтобы Spotify разрешил Aria играть музыку самой.
spotify-device-aria = Aria (этот компьютер)
spotify-player-starting = Запускается плеер Spotify в Aria…
spotify-player-reconnect = Переподключите Spotify в настройках, чтобы он играл в Aria.
spotify-player-premium = Чтобы Spotify играл в Aria, нужен Spotify Premium.
spotify-player-no-drm = Этот браузер не может играть Spotify (в нём нет модуля защиты от копирования). Установите Chrome или Edge.
spotify-player-failed = Spotify не смог сыграть это в Aria.
spotify-player-timeout = Плеер Spotify в Aria не запустился.
spotify-player-launch-failed = Aria не смогла открыть окно плеера.
spotify-player-closed = Окно плеера Spotify закрыто; нажмите «Играть», чтобы открыть его снова.
spotify-player-tab = Edge или Chrome не найден, поэтому Spotify играет во вкладке вашего браузера. Нажмите там «Старт».
spotify-player-autoplay = Нажмите «Старт» на странице Spotify в Aria, чтобы музыка заиграла.
player-page-title = Aria — Spotify
player-page-playing = Aria играет Spotify в этом окне. Его можно свернуть; если закрыть — Spotify в Aria остановится.
player-page-starting = Запуск…
player-page-ready = Готово
player-page-failed = Spotify не смог здесь запуститься. Причину покажет Aria.
player-page-start = Старт
player-page-reload = Закройте это окно и нажмите «Играть» в Aria.
