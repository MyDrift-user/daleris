# Daleris in other languages

Daleris shows its text in English, German or Italian (Settings > Appearance > Language, or the `Language` policy).
The English in the code is the key; the catalogs in `de/` and `it/` hold the translations, one file per part of the
app.

## Marking text

| What | How |
|---|---|
| Plain text | `tr("Open")` |
| Text with values in it | `trf!("Copying {name} to {folder}", name = n, folder = f)`, named `{values}` only |
| Text for a count | `trn!(count, "{n} item", "{n} items")`, the first for exactly one; `{n}` is the count, written as the language writes numbers |
| Text kept in a table | `tx("Date modified")` where it's written, `tr(entry)` where it's shown |
| Text in a table that reaches the screen as a `String` | `tr_dyn(&s)` |
| The launcher (`crates/open`), which can't use the catalogs | `three("English", "Deutsch", "Italiano")` |
| A string that isn't text for a person | the line ends with `// i18n-ignore: <why>` |

Never build a sentence from pieces (`tr("Copy") + " " + tr("to")`): word order differs between languages. Give the
whole sentence with `{values}`. Sizes, counts and dates go through `util::fmt_size`, `fmt_count` and `fmt_filetime`,
which write them as the language does (2,5 GB, 1.234, 02.11.2026 in German).

`cargo test i18n` checks it all: every marked text has a German and an Italian translation with the same `{values}`,
no translation is left over, and no string that reads like words to a person is left unmarked.
`DALERIS_I18N_ONLY=src/app/menu.rs cargo test no_text_is_left_unmarked` looks at some files only.

## How Daleris speaks

Short, plain sentences, as in the English. Say what happens and what to do; no "please", no exclamation marks.

- **German:** "Sie", as Windows does. Infinitives for commands in menus and buttons ("Ordner öffnen"). Windows's own
  words for Windows's own things. Keys as written on German keyboards: Strg, Umschalt, Alt, Entf, Eingabetaste,
  Rücktaste; "Strg+C", "Strg+Umschalt+T".
- **Italian:** the informal "tu" in imperatives, as Microsoft's Italian does ("Scegli una cartella"); infinitives in
  menus and buttons ("Apri cartella"). Keys: Ctrl, Maiusc, Alt, Canc, Invio, Backspace; "Ctrl+C", "Ctrl+Maiusc+T".
- Keep "Daleris", "Windows", "Explorer" (as a name: "wie im Explorer", "come in Esplora file"), "OneDrive", "Mica",
  "regex", file names and extensions as they are.
- Keep `{values}` exactly as in the English; move them wherever the sentence needs them.
- Keep the punctuation of the English at the ends of lines (a full stop or none, "…" for something that asks first).

## Glossary

Windows 11 Explorer's own words wherever it has one.

| English | German | Italian |
|---|---|---|
| File Explorer / Windows Explorer | Datei-Explorer / Windows-Explorer | Esplora file |
| This PC | Dieser PC | Questo PC |
| Home | Start | Home |
| Quick access | Schnellzugriff | Accesso rapido |
| Desktop | Desktop | Desktop |
| Documents | Dokumente | Documenti |
| Downloads | Downloads | Download |
| Pictures | Bilder | Immagini |
| Music | Musik | Musica |
| Videos | Videos | Video |
| Network | Netzwerk | Rete |
| Cloud drives | Cloudlaufwerke | Unità cloud |
| Recycle Bin | Papierkorb | Cestino |
| Local Disk | Lokaler Datenträger | Disco locale |
| USB Drive / DVD Drive / Network Drive | USB-Laufwerk / DVD-Laufwerk / Netzlaufwerk | Unità USB / Unità DVD / Unità di rete |
| drive | Laufwerk | unità |
| folder / file / item(s) | Ordner / Datei / Element(e) | cartella / file / elemento (elementi) |
| tab | Registerkarte | scheda |
| New tab | Neue Registerkarte | Nuova scheda |
| window / New window | Fenster / Neues Fenster | finestra / Nuova finestra |
| Settings | Einstellungen | Impostazioni |
| sidebar | Seitenleiste | barra laterale |
| ribbon bar | Befehlsleiste | barra dei comandi |
| address bar | Adressleiste | barra degli indirizzi |
| title bar | Titelleiste | barra del titolo |
| tab bar | Registerkartenleiste | barra delle schede |
| preview pane | Vorschaufenster | riquadro di anteprima |
| right-click menu | Kontextmenü | menu di scelta rapida |
| Open / Open with | Öffnen / Öffnen mit | Apri / Apri con |
| Cut / Copy / Paste | Ausschneiden / Kopieren / Einfügen | Taglia / Copia / Incolla |
| Rename / Delete | Umbenennen / Löschen | Rinomina / Elimina |
| Properties | Eigenschaften | Proprietà |
| New / New folder | Neu / Neuer Ordner | Nuovo / Nuova cartella |
| Copy path | Als Pfad kopieren | Copia come percorso |
| Undo / Redo | Rückgängig / Wiederholen | Annulla / Ripeti |
| Select all | Alles auswählen | Seleziona tutto |
| Refresh | Aktualisieren | Aggiorna |
| Back / Forward / Up | Zurück / Vorwärts / Nach oben | Indietro / Avanti / Su |
| Search / Filter | Suchen / Filtern | Cerca / Filtra |
| Sort by | Sortieren nach | Ordina per |
| Name / Date modified / Date created / Type / Size | Name / Änderungsdatum / Erstelldatum / Typ / Größe | Nome / Ultima modifica / Data creazione / Tipo / Dimensione |
| View / Details / Large icons | Anzeigen / Details / Große Symbole | Visualizza / Dettagli / Icone grandi |
| Hidden items | Ausgeblendete Elemente | Elementi nascosti |
| File name extensions | Dateinamenerweiterungen | Estensioni nomi file |
| File folder | Dateiordner | Cartella di file |
| Map network drive | Netzlaufwerk verbinden | Connetti unità di rete |
| Disconnect | Trennen | Disconnetti |
| Eject | Auswerfen | Espelli |
| Pin / Unpin (sidebar) | Anheften / Lösen | Aggiungi / Rimuovi (da Accesso rapido) |
| Theme / Light / Dark | Design / Hell / Dunkel | Tema / Chiaro / Scuro |
| Accent colour | Akzentfarbe | Colore principale |
| Icon pack | Symbolpaket | Pacchetto di icone |
| Row height / Compact | Zeilenhöhe / Kompakt | Altezza righe / Compatta |
| Managed by your organization | Von Ihrer Organisation verwaltet | Gestito dalla tua organizzazione |
| administrator | Administrator | amministratore |
| update(s) | Update(s) | aggiornamento/i |
| copy / move (an operation) | Kopieren / Verschieben | copia / spostamento |
| Cancel / OK / Close | Abbrechen / OK / Schließen | Annulla / OK / Chiudi |
| Try again | Wiederholen | Riprova |
| Sign in | Anmelden | Accedi |
| share (network) | Freigabe | condivisione |
| server | Server | server |
| shortcut (.lnk) | Verknüpfung | collegamento |
| keyboard shortcut | Tastenkombination | scelta rapida da tastiera |
