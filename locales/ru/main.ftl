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
notice-dismiss = Закрыть

## Library notices (toasts)

# $count (Number) - how many files and folders the scan could not read
library-scan-skipped = { $count ->
    [one] Не удалось прочитать { $count } файл или папку, при сканировании они пропущены.
    [few] Не удалось прочитать { $count } файла или папки, при сканировании они пропущены.
   *[other] Не удалось прочитать { $count } файлов или папок, при сканировании они пропущены.
}
# $folder (String) - the last part of the folder's path, e.g. Music
library-folder-unavailable = Папка { $folder } недоступна, или все её файлы перемещены либо удалены. Треки сохранены, а папку можно удалить в настройках.
library-folder-nested = Эта папка находится внутри папки, которая уже есть в библиотеке.
library-folder-duplicate = Эта папка уже есть в библиотеке.
library-recovered = База данных библиотеки была повреждена, поэтому создана новая. Старый файл сохранён как резервная копия.
library-newer = Библиотека сохранена более новой версией Aria, поэтому она отключена. Обновите Aria, чтобы пользоваться ею.
library-unavailable = Не удалось открыть библиотеку, поэтому она пока отключена. Остальные функции Aria продолжают работать.
library-action-failed = В библиотеке произошла ошибка, и действие не завершено. Ваши музыкальные файлы не изменены.

## Playback notices (toasts)

# $title (String) - the track's title, e.g. Blue in Green
playback-track-failed = Не удалось воспроизвести «{ $title }», трек пропущен.
playback-stopped-after-failures = Воспроизведение остановлено: несколько треков подряд воспроизвести не удалось.
playback-device-fallback = Аудиоустройство недоступно, поэтому Aria переключилась на другое.
playback-no-device = Нет доступного аудиоустройства, поэтому воспроизведение приостановлено.

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

## Language names: endonyms, identical in every locale

lang-en = English
lang-ru = Русский
lang-hy = Հայերեն
