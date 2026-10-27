<p align="center"><img src="logo.svg" width="112" alt="logo rootwhy"></p>

# rootwhy

**sudo z okienkiem na hasło, które mówi _po co_ — i wyjaśnia każdy fragment polecenia.**

[English](README.md)

Gdy agent AI (albo skrypt) potrzebuje roota, zwykle dostajesz gołe pytanie o hasło albo jednowierszowe polecenie do
rozszyfrowania. `rootwhy` pokazuje okienko z:

- **powodem**, napisanym przez proszącego (np. agenta),
- **poleceniem podzielonym na wiersze i pokolorowanym** — polecenie, opcje, wartości, ścieżki,
- **wyjaśnieniem każdego fragmentu po najechaniu**: co deklaruje proszący, wbudowany opis prostym językiem, fakty
  sprawdzone w tej chwili (czy plik istnieje, kto jest właścicielem, prawa, rozmiar, czy cel zostanie nadpisany)
  i opis opcji z `man`,
- katalogiem roboczym i łańcuchem procesów, które proszą.

Hasło idzie z okienka prosto do `sudo`. Proszący go nie widzi.

![okienko rootwhy](docs/dialog-pl.png)

Najechanie na `755` pokazuje np. *prawa rwxr-xr-x: właściciel odczyt, zapis, uruchamianie; grupa odczyt,
uruchamianie; pozostali odczyt, uruchamianie*, a na cel — czy istnieje i czym jest `/usr/local/bin`.

## Użycie

```bash
rootwhy -r "Restart Ollamy po zmianie pliku usługi." systemctl restart ollama

rootwhy -r "Instalacja nocnego skryptu kopii dla całego systemu, żeby timer mógł go uruchamiać jako root." \
        -e "755=skrypt musi być wykonywalny" \
        -e "nightly-backup.sh=nowa wersja z poprawioną ścieżką" \
        -- install -m 755 -o root -g root ./build/nightly-backup.sh /usr/local/bin/nightly-backup.sh
```

| Opcja | |
| --- | --- |
| `-r, --reason TEKST` | po co jest polecenie, 10–300 znaków (wymagane) |
| `-e, --explain FRAGMENT=TEKST` | wyjaśnienie po najechaniu na pasujący fragment; można powtarzać. Pasuje do całego argumentu, nazwy opcji albo końca ścieżki. Wyjaśnienia bez dopasowania są wypisane pod poleceniem |
| `--log-dir KATALOG` | katalog dziennika (domyślnie `$XDG_STATE_HOME/rootwhy`, czyli `~/.local/state/rootwhy`; także `ROOTWHY_LOG_DIR`) |
| `--preview` | tylko pokaż okienko, niczego nie uruchamiaj |

Kod wyjścia: kod polecenia, `130` po anulowaniu, `2` przy złym użyciu. Po błędnym haśle `sudo` pyta ponownie
(do 3 razy), a okienko o tym mówi.

### Dla agentów AI

Każ agentowi używać `rootwhy` zamiast `sudo`, np. w `CLAUDE.md` / `AGENTS.md`:

```markdown
Polecenia wymagające roota — tylko przez `rootwhy -r "Co i po co, 1–2 zdania." [-e FRAGMENT=PO_CO …] -- polecenie …`,
nigdy `sudo` wprost. Uprzedź użytkownika, że pojawi się okienko z hasłem. Kod 130 = anulował — nie ponawiaj bez pytania.
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
- **Nic nie działa przed zgodą.** Wyjaśnienia pochodzą z wbudowanej tabeli, `man` (nie uruchamia polecenia), `stat`
  ścieżek i `systemctl show` (opis jednostki).
- **Deklaracje są oznaczone.** Powód i teksty `-e` pochodzą od proszącego i mają etykietę *deklaracja, niesprawdzona*;
  fakty sprawdzone przez rootwhy — *Sprawdzone teraz*. Nieuczciwy proszący może skłamać w powodzie — zatwierdzasz
  samo polecenie.
- **Dziennik bez haseł:** data, powód, polecenie i kod wyjścia, po jednym wierszu, zapisany przed otwarciem okienka.

Ograniczenia: `rootwhy` nie wie, co zrobi skrypt po uruchomieniu — tylko jaki to plik i kto może go zmienić (ostrzega,
gdy program lub skrypt nie należy do roota albo inni mogą go zmieniać). Reguły `sudo` bez hasła (`NOPASSWD`) i metody
PAM, np. czytnik linii papilarnych, całkowicie pomijają okienko.

## Instalacja

Wymagania: Linux, `sudo`, Python 3.9+, a dla pełnego okienka GTK 4 z PyGObject. Bez GTK 4 używa `kdialog` albo
`zenity` (powód i polecenie w wierszach, bez kolorów i dymków).

```bash
# Fedora
sudo dnf install python3-gobject gtk4
# Debian, Ubuntu, Kubuntu
sudo apt install python3-gi gir1.2-gtk-4.0

git clone https://github.com/DragonKing026/rootwhy.git ~/.local/share/rootwhy
ln -s ~/.local/share/rootwhy/rootwhy ~/.local/bin/rootwhy
rootwhy --version
```

Działa w KDE Plasma i GNOME, na Waylandzie i X11. Aktualizacja: `git -C ~/.local/share/rootwhy pull`. Odinstalowanie:
usuń dowiązanie i katalog (oraz `~/.local/state/rootwhy` z dziennikiem).

## Język

Interfejs według `LANG` / `LC_MESSAGES`: polski dla `pl_*`, w pozostałych angielski. Wymuszenie: `ROOTWHY_LANG=en`
albo `ROOTWHY_LANG=pl`. Opisy z `man` są w języku zainstalowanych stron man.

## Rozwój

```bash
python3 -m unittest discover tests        # model polecenia, wyjaśnienia, tłumaczenia (bez GTK i sudo)
./rootwhy --preview -r "Test okienka." -- install -m 644 a /etc/b
./rootwhy --screenshot out.png -r "Test okienka." -- systemctl restart foo   # okno do pliku PNG
```

Program jest celowo jednym plikiem — dostaje Twoje hasło, więc powinno się go dać przeczytać za jednym posiedzeniem.

## Licencja

[GPL-3.0-or-later](LICENSE). Wolno używać, zmieniać i sprzedawać, ale wszystko, co rozpowszechniasz na jego podstawie,
musi zostać na tej samej licencji, z kodem źródłowym.
