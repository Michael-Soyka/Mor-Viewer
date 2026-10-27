# Mor Viewer

Просмотрщик ресурсов для игры "Мор. Утопия" (Pathologic).

<img width="1712" height="863" alt="image" src="https://github.com/user-attachments/assets/d0e05b74-aa4f-4eac-b8f0-cf11f8c6f5f9" />
<p align="center"><b>City view</b></p>

| Model view | Interior view |
| -             | -             |
| <img width="1712" height="863" alt="model-view" src="https://github.com/user-attachments/assets/99510ae2-e516-4c82-80ac-cbf484d434a7" /> | <img width="1712" height="863" alt="interior-view" src="https://github.com/user-attachments/assets/db308130-1251-4c42-bfb5-18d61dd26d9c" /> |

| Textures view | Fonts view |
| -             | -             |
| <img width="1712" height="863" alt="texture-view" src="https://github.com/user-attachments/assets/e845f6ad-b879-4fc5-bef9-7b859479103b" /> | <img width="1712" height="863" alt="font-view" src="https://github.com/user-attachments/assets/388a4d21-521e-479c-b09e-e4b0c3a52b57" /> |

## Возможности

| Тип | Расширения | Что может |
|---|---|---|
| Текстуры | `.tex`, PNG, JPEG, TGA | просмотр по каналам и масштабу, экспорт в PNG |
| Шрифты | `.ft` | атлас глифов и пример текста |
| Модели | `.mesh`, `.mms`, `.mma`, `.lsh` | 3D-просмотр, анимации персонажей, экспорт в glTF |
| Сцены | `.isc`, `.osc` | интерьеры и уровни целиком: карты освещения, источники света, вода, небо и время суток, навигационные пути |
| Карты | `.map` | карта с подписями |
| Тексты | `.xml`, `.txt`, `.json` | подсветка синтаксиса |
| Звук | `.ogg` | воспроизведение |
| Прочее | любые | hex-просмотр, экспорт как есть |

Интерфейс на русском и английском.

## Минимальные требования

| Пункт | Описание |
|---|---|
| ОС | Windows 10 и новее (64 бит), Linux, macOS 11 и новее |
| Графический API | Vulkan (Windows, Linux) или Metal (macOS); OpenGL и DirectX 12 не поддерживаются |
| Видеокарта | с драйвером Vulkan и текстурами до 8192 пикселей: NVIDIA GeForce GTX 600, AMD Radeon HD 7000, Intel HD Graphics 500 и новее; на macOS - любой Mac с Metal |
| Данные | файлы игры "Мор. Утопия" (2005): папка установки или её VFS-архивы |

## Запуск

```sh
mor-viewer [папка_с_данными | архив.vfs | файл_игры]
```

Без аргумента берётся путь из переменной окружения `PATHOLOGIC_DATA`.

Отдельные файлы игры (`.mesh`, `.tex`, `.ft`, `.isc` и остальные) открываются напрямую, без архива: через "Файл → Открыть файлы...", аргументом командной строки или перетаскиванием в окно. *Каждый открывается в своей вкладке, в дереве их не будет. Модель ищет свои текстуры среди открытых файлов и в архивах: если текстура открыта вместе с моделью, она наложится, иначе модель будет без неё.*

По умолчанию из папки `data` открываются только архивы `.vfs`. Распакованные файлы - опция "Загружать распакованные папки" в настройках, на странице "Игра":

- **папка `data`** - вместе с архивами открываются все папки рядом с ними, например и `Actors.vfs`, и распакованная `Actors/`. Когда модель или сцена ссылается на файл по имени, он сначала ищется в распакованных папках, потом в архивах, так что правленые файлы подхватываются сразу;
- **папка без архивов** - открывается целиком как распакованная: `.mesh`, `.tex`, `.ft` и остальное лежат в ней как есть.

## Сборка

Парсеры форматов берутся из репозитория [Mor-parser](https://github.com/Michael-Soyka/Mor-parser), Cargo подтянет их сам:

```sh
cargo run --release
```

Поддерживается локальная разработка, достаточно подставить в `.cargo/config.toml`:

```toml
[patch."https://github.com/Michael-Soyka/Mor-parser"]
morft    = { path = "../mor-parsers/crates/morft" }
mormesh  = { path = "../mor-parsers/crates/mormesh" }
morscene = { path = "../mor-parsers/crates/morscene" }
mortex   = { path = "../mor-parsers/crates/mortex" }
morvfs   = { path = "../mor-parsers/crates/morvfs" }
```

## Структура

```
src/
  main.rs, app.rs   запуск и главное окно
  settings.rs       настройки
  i18n.rs           локализация (тексты в locales/)
  data/             архивы, определение типа файлов, декодирование, экспорт
  render/           GPU: модели, отсечение, вода, небо, анимация, загрузка сцен
    shaders/        WGSL-шейдеры
  ui/               тема, виджеты, иконки, заголовок окна
  viewer/           просмотрщики по типам файлов
    model/          3D-просмотрщик: камера, меню, оверлеи
```
