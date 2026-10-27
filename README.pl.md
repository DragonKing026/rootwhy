<p align="center"><img src="logo.svg" width="112" alt="logo rootwhy"></p>

# rootwhy

**sudo z okienkiem na hasło, które mówi _po co_ — i pokazuje, co robi każdy fragment polecenia.**

[English](README.md)

Gdy agent AI (albo skrypt) potrzebuje roota, zwykle dostajesz gołe pytanie o hasło albo jednowierszowe polecenie do
rozszyfrowania. `rootwhy` pokazuje okienko z:

- **powodem**, napisanym przez proszącego (np. agenta),
- **poleceniem podzielonym na wiersze i pokolorowanym** — polecenie, opcje, wartości, ścieżki,
- **po najechaniu: co robi każdy fragment — z podręcznika Twojego systemu** (`man`, po polsku, jeśli jest
  tłumaczenie), a do tego ostrzeżenie ⚠, gdy coś wymaga uwagi: plik docelowy zostanie zastąpiony, program lub skrypt
  może zmienić ktoś poza rootem, ścieżka nie istnieje, tryb setuid, nieznany użytkownik,
- katalogiem roboczym i programami, przez które przyszła prośba (np. `code → claude → zsh`).

Nic nie jest wymyślone: rootwhy nie ma własnej tabeli opisów i nie bierze wyjaśnień od proszącego. Fragment, o którym
system nic nie wie, po prostu nie ma dymka.

Hasło idzie z okienka prosto do `sudo`. Proszący go nie widzi.

![okienko rootwhy](docs/dialog-pl.png)

Najechanie na `-m` pokazuje np. *set permission mode (as in chmod), instead of rwxr-xr-x* — tekst z `man install`.
Najechanie na cel pokazuje *⚠ plik istnieje — zostanie zastąpiony*, jeśli tak jest.

## Użycie

```bash
rootwhy -r "Restart Ollamy po zmianie pliku usługi." systemctl restart ollama

rootwhy -r "Instalacja nocnego skryptu kopii dla całego systemu, żeby timer mógł go uruchamiać jako root." \
        -- install -m 755 -o root -g root ./build/nightly-backup.sh /usr/local/bin/nightly-backup.sh
```

| Opcja | |
| --- | --- |
| `-r, --reason TEKST` | po co jest polecenie, 10–300 znaków (wymagane) |
| `--log-dir KATALOG` | katalog dziennika (domyślnie `$XDG_STATE_HOME/rootwhy`, czyli `~/.local/state/rootwhy`; także `ROOTWHY_LOG_DIR`) |
| `--preview` | tylko pokaż okienko, niczego nie uruchamiaj |

Kod wyjścia: kod polecenia, `130` po anulowaniu, `2` przy złym użyciu. Po błędnym haśle `sudo` pyta ponownie
(do 3 razy), a okienko o tym mówi.

### Dla agentów AI — wtyczka Claude Code

Repozytorium jest też wtyczką Claude Code: **skill**, który mówi agentowi, kiedy i jak używać rootwhy, oraz **hook**,
który zatrzymuje `sudo`, `pkexec`, `doas`, `run0` i `su` wywołane wprost i odsyła do rootwhy.

```text
/plugin marketplace add DragonKing026/rootwhy
/plugin install rootwhy@rootwhy
```

Wtyczka nie instaluje programu — zrób to najpierw (Instalacja). Dla innych agentów ta sama reguła w `AGENTS.md`:

```markdown
Polecenia wymagające roota — tylko przez `rootwhy -r "Co i po co, 1–2 zdania." -- polecenie …`, nigdy `sudo`
wprost. Uprzedź użytkownika, że pojawi się okienko z hasłem. Kod 130 = anulował — nie ponawiaj bez pytania.
```

## Bezpieczeństwo

- **Każde wywołanie pyta o hasło** (`sudo -k`) — bez korzystania z wcześniejszej zgody.
- **Tryb hasła tylko pod prawdziwym sudo.** `rootwhy` jest swoim własnym `SUDO_ASKPASS`. W tym trybie pokaże okienko
  tylko wtedy, gdy rodzicem jest `sudo` uruchomione przez Ciebie: nazwa procesu `sudo`, rzeczywisty UID Twój,
  efektywny UID 0 (ma go tylko program setuid roota) i, gdy jest widoczny, plik wykonywalny roota z bitem setuid.
  Odziedziczone środowisko ani podrobione `sudo` nie zamienią go w okienko, które odda hasło temu, kto je uruchomił.
- **Widzisz to, co się wykona.** Polecenie w okienku pochodzi z `/proc/PID/cmdline` samego sudo, nie z danych od
  proszącego: wywołanie `sudo` wprost z rootwhy jako programem pytającym i niewinnym opisem i tak pokaże prawdziwe
  polecenie. Nazwa polecenia jest zamieniana na ścieżkę bezwzględną, zanim dostanie ją sudo. Nic nie jest skracane;
  argumenty są w cudzysłowach jak w powłoce, a znaki niewidoczne i sterujące kierunkiem tekstu są pokazane jako
  np. `$'\u202e'`.
- **Czysty proces okienka.** Python działa z `-I`, a okienko zachowuje tylko zmienne ekranu, sesji i języka:
  `PYTHONPATH`, `GIO_EXTRA_MODULES`, `GTK_*`, `QT_PLUGIN_PATH` itp. nie wczytają obcego kodu do procesu z hasłem.
- **Nic nie działa przed zgodą.** Opisy pochodzą tylko z `man` (nie uruchamia polecenia — `--help` nigdy nie jest
  wywoływane), a ostrzeżenia ze `stat` ścieżek.
- **Proszący nie wpisze nic do dymków.** Od proszącego pochodzi tylko powód. Nieuczciwy proszący może skłamać
  w powodzie — zatwierdzasz samo polecenie.
- **Program pokazany z nazwy** (`ls`) tylko wtedy, gdy `sudo` znajdzie po tej nazwie ten sam plik i jest on bezpieczny;
  inaczej pełna ścieżka i ostrzeżenie.
- **Dziennik bez haseł:** data, powód, polecenie i kod wyjścia, po jednym wierszu, zapisany przed otwarciem okienka.

Ograniczenia: `rootwhy` nie wie, co zrobi skrypt po uruchomieniu — tylko jaki to plik i kto może go zmienić (ostrzega,
gdy program lub skrypt nie należy do roota albo inni mogą go zmieniać). Reguły `sudo` bez hasła (`NOPASSWD`) i metody
PAM, np. czytnik linii papilarnych, całkowicie pomijają okienko.

## Instalacja

Jeden plik, nic więcej do instalowania. Pobierz go z
[najnowszego wydania](https://github.com/DragonKing026/rootwhy/releases/latest), sprawdź sumę i nadaj prawo
uruchamiania:

```bash
mkdir -p ~/.local/bin && cd ~/.local/bin
curl -fsSLO https://github.com/DragonKing026/rootwhy/releases/latest/download/rootwhy
curl -fsSL https://github.com/DragonKing026/rootwhy/releases/latest/download/SHA256SUMS | sha256sum -c
chmod +x rootwhy
rootwhy --version
```

Korzysta z tego, co pulpit Linuksa już ma: `sudo`, Python 3 i powiązania GTK dla Pythona (PyGObject) — GTK 4, a gdy
go brak, GTK 3. Gdy nie ma żadnego, używa `kdialog` (KDE) albo `zenity` (GNOME): powód i polecenie w wierszach, bez
kolorów i dymków. Działa w KDE Plasma i GNOME, na Waylandzie i X11.

Aktualizacja: pobierz nowe wydanie tak samo. Odinstalowanie: `rm ~/.local/bin/rootwhy` (oraz `~/.local/state/rootwhy`
z dziennikiem).

## Język

Kilka stałych napisów okienka (przyciski, ostrzeżenia) według `LANG` / `LC_MESSAGES`: polski dla `pl_*`, w pozostałych
angielski; wymuszenie `ROOTWHY_LANG=en` albo `ROOTWHY_LANG=pl`. Opisy są w języku zainstalowanych stron man — po
polsku po instalacji tłumaczeń z dystrybucji (np. `man-pages-pl`).

## Rozwój

```bash
python3 -m unittest discover tests        # model polecenia, odczyt man, ostrzeżenia, tłumaczenia (bez GTK i sudo)
./rootwhy --preview -r "Test okienka." -- install -m 644 a /etc/b
./rootwhy --screenshot out.png -r "Test okienka." -- systemctl restart foo   # okno do pliku PNG
ROOTWHY_GTK=3 ./rootwhy --preview -r "Test GTK 3." -- true                  # wymuszenie okienka GTK 3
```

Logo jest wbudowane w program (`LOGO_SVG`, test pilnuje zgodności z `logo.svg`), więc wydanie to jeden plik.

Program jest celowo jednym plikiem — dostaje Twoje hasło, więc powinno się go dać przeczytać za jednym posiedzeniem.
Testy i `ruff` uruchamiają się przy każdym pushu (`.github/workflows/test.yml`).

## Licencja

[GPL-3.0-or-later](LICENSE). Wolno używać, zmieniać i sprzedawać, ale wszystko, co rozpowszechniasz na jego podstawie,
musi zostać na tej samej licencji, z kodem źródłowym.
