# Needs native-speaker review before 1.0

### Aria UI strings: Eastern Armenian.
### Translation of locales/en/main.ftl: keep the same message ids and placeables.

## Navigation: sidebar entries, page headings and history buttons

nav-home = Գլխավոր
nav-search = Որոնում
nav-library = Իմ գրադարանը
nav-liked = Հավանած երգեր
nav-playlists = Երգացանկեր
nav-albums = Ալբոմներ
nav-artists = Կատարողներ
nav-local = Տեղային ֆայլեր
nav-settings = Կարգավորումներ
nav-back = Հետ
nav-forward = Առաջ

## Empty states

# $phase (Number) - the roadmap phase that delivers the page, e.g. 2
empty-phase-pending = Հասանելի կլինի { $phase }-րդ փուլում
empty-nothing-playing = Ոչինչ չի նվագարկվում
empty-right-panel = Հերթը դատարկ է

## Player controls (tooltips)

player-play = Նվագարկել
player-previous = Նախորդ երգը
player-next = Հաջորդ երգը
player-volume = Ձայնի ուժգնություն
player-devices = Սարքեր
player-queue = Հերթ
player-lyrics = Երգի բառեր
# $phase (Number) - the roadmap phase that delivers the control, e.g. 2
player-coming-in = Կհայտնվի { $phase }-րդ փուլում

## Settings page

settings-general = Ընդհանուր
settings-language = Լեզու
settings-language-system = Ըստ համակարգի
settings-theme = Թեմա
settings-theme-dark = Մուգ
settings-theme-light = Բաց
settings-theme-system = Ըստ համակարգի
settings-accent-dark = Մուգ թեմայի շեշտադրման գույն
settings-accent-light = Բաց թեմայի շեշտադրման գույն
settings-accent-reset = Վերակայել
settings-minimize-to-tray = Ծալել համակարգային սկուտեղում
settings-minimize-to-tray-hint = Պատուհանը կծալվի համակարգային սկուտեղում, այլ ոչ թե առաջադրանքների վահանակում։ Կգործի 4-րդ փուլից սկսած։
settings-advanced = Լրացուցիչ
settings-log-level = Մատյանավորման մակարդակ
settings-log-level-error = Սխալներ
settings-log-level-warn = Զգուշացումներ
settings-log-level-info = Տեղեկություններ
settings-log-level-debug = Վրիպազերծում
settings-log-level-trace = Հետագծում
settings-open-config = Բացել կարգավորումների պանակը
settings-open-logs = Բացել մատյանների պանակը
settings-reset = Վերակայել կարգավորումները
settings-reset-confirm-title = Վերակայե՞լ բոլոր կարգավորումները
settings-reset-confirm-body = Բոլոր կարգավորումները կվերադառնան լռելյայն արժեքներին։ Այս գործողությունը հնարավոր չէ հետարկել։
settings-cancel = Չեղարկել
settings-confirm = Հաստատել
settings-about = Ծրագրի մասին
settings-version = Տարբերակ
settings-license = Լիցենզիա
settings-third-party = Երրորդ կողմի բաղադրիչների լիցենզիաները նշված են THIRD_PARTY_NOTICES.md ֆայլում

## Notices (toasts)

notice-settings-recovered = Չհաջողվեց կարդալ կարգավորումների ֆայլը, ուստի վերականգնվել են լռելյայն արժեքները։ Հին ֆայլի պահուստային պատճենը պահպանվել է։
notice-settings-readonly = Կարգավորումների ֆայլը հասանելի չէ, ուստի օգտագործվում են լռելյայն արժեքները։ Փոփոխությունները կարող են չպահպանվել։
notice-settings-save-failed = Չհաջողվեց պահպանել կարգավորումները։ Նոր փորձ կկատարվի հաջորդ փոփոխության ժամանակ։
notice-open-folder-failed = Չհաջողվեց բացել պանակը։
notice-dismiss = Փակել

## Library notices (toasts)

# $count (Number) - how many files and folders the scan could not read
library-scan-skipped = { $count ->
    [one] Չհաջողվեց կարդալ { $count } ֆայլ կամ պանակ, և այն բաց է թողնվել։
   *[other] Չհաջողվեց կարդալ { $count } ֆայլ կամ պանակ, և դրանք բաց են թողնվել։
}
# $folder (String) - the last part of the folder's path, e.g. Music
library-folder-unavailable = { $folder } պանակը հասանելի չէ, կամ դրա բոլոր ֆայլերը տեղափոխվել կամ ջնջվել են։ Երգերը պահպանված են, իսկ պանակը կարող եք հեռացնել Կարգավորումներում։
library-folder-nested = Այս պանակը գտնվում է մի պանակի մեջ, որն արդեն գրադարանում է։
library-folder-duplicate = Այս պանակն արդեն գրադարանում է։
library-recovered = Գրադարանի տվյալների բազան վնասված էր, ուստի ստեղծվել է նորը։ Հին ֆայլը պահպանվել է որպես պահուստային պատճեն։
library-newer = Գրադարանը պահպանվել է Aria-ի ավելի նոր տարբերակով, ուստի այն անջատված է։ Թարմացրեք Aria-ն՝ այն օգտագործելու համար։
library-unavailable = Չհաջողվեց բացել գրադարանը, ուստի այն առայժմ անջատված է։ Aria-ի մնացած գործառույթները շարունակում են աշխատել։
library-action-failed = Գրադարանում սխալ տեղի ունեցավ, և գործողությունը չավարտվեց։ Ձեր երաժշտական ֆայլերը չեն փոխվել։

## Fatal startup errors (a message box shown before Aria quits)

error-startup-title = Չհաջողվեց գործարկել Aria-ն
error-startup-window = Aria-ին չհաջողվեց բացել իր պատուհանը։

## Language names: endonyms, identical in every locale

lang-en = English
lang-ru = Русский
lang-hy = Հայերեն
