# Changelog

What changed for people using Daleris, compared with the version before. A release's notes on GitHub are its section here.

## 0.0.6

- Network drives are left alone unless asked: subfolders aren't searched and folder sizes aren't counted there by default, each with a switch and a policy, and the status bar offers a search of the subfolders once
- Searches and folder sizes read gently: low disk priority, one folder at a time on hard disks, USB drives and servers, and at most 20 folders a second on servers
- Files can be dragged out of Daleris onto the desktop, Explorer and other apps, such as a browser's upload area or a mail, also onto windows in front of Daleris and onto the taskbar to switch apps
- Text copied from the address bar or search box reaches the Windows clipboard, and both have a right-click menu with Cut, Copy, Paste and Select all
- The Win + E hook never keeps back a plain E any more, also after Win was released on the lock screen or a UAC prompt
- Win + E starts Daleris directly, one program start fewer
- A closed window leaves the screen at once, before Daleris saves and the graphics driver lets go
- Dragging the window is as smooth as Windows makes it, whatever folder it shows: nothing is drawn again while it moves
- New drives and USB sticks show up within a quarter of a second, also while mapped network drives are slow to answer, and a stick Windows is still mounting gets its name and size as soon as it can be read
- "Keep folders above files" applies to every folder that wasn't switched in its own Sort menu, also ones sorted some other way before
- Typing a name in the list scrolls to the item it selects, however far down it is
- Folders with many items draw faster: who you are on the network is looked up once instead of for every row
- The status of "Open and Save dialogs use Daleris" says that browsers and other sandboxed apps keep Windows's own dialog
- The filter panel only offers extensions of items the list can show
- A disconnected drive is named by its own letter, and a disconnected share by its name, instead of "Drive ?:"
- No middle dots or arrow characters anywhere in the window
- Phones and cameras connected by USB show up in the sidebar and This PC as they're connected, and open like folders: their storages, folders and files with Windows's icons, sizes and dates, read without holding up the window. Files are copied to and from them by dragging or copy and paste, and deleted, renamed and opened there, with Windows's own progress

## 0.0.5

### Looks
- Explorer's look, on by default on Windows: compact rows, Mica behind the tab bar and address bar on Windows 11, and the Windows icons drawn as sharp as Explorer's
- Explorer's look is a preset you apply in Settings, setting each of its parts, which stay settings and policies of their own
- The theme follows Windows's light or dark mode by default
- The logo and name in the title bar can be the settings button, with Settings as their tab
- Right-clicking the title bar gives Windows's window menu

### Languages
- German and Italian, complete, beside English
- Follows the language Windows is shown in, or chosen in Settings, or set by policy
- Numbers and dates written the way each language writes them
- One MSI in all three languages, following Windows's language, and policy templates in all three

### Files
- Copy, move and delete run in Daleris's own window, with a progress card, speed and time left
- Items on their way show a progress ring, a badge and a tooltip in the list
- Who has a file open shows next to it: programs on this PC, Office and LibreOffice lock files, and other people's Daleris on network shares
- Drag and drop onto folders, tabs, the sidebar and any part of the path, showing beforehand whether it can go there, needs administrator rights, or can't, and why
- Drag tabs to reorder them, into a window of their own, or into another window
- "Open in new window", and starting Daleris again opens a new window (or a new tab, as a setting)
- The New menu offers what Explorer's does

### Tabs and search
- Choose where a new tab opens: This PC by default, Home, a folder, or the same folder; Shift with + or Ctrl+Shift+T always opens the same folder
- Choose what search never finds: everything listed except cloud folders is left out by default
- "Search subfolders too" is one setting, shared by Settings and the search box
- The search box's subfolders, regular expression and help buttons can be hidden
- Dotfiles and dotfolders are hidden by default on Windows

### Drives and network
- Drives appear and disappear as they're connected, with an optional tab for each new one
- Unlock a BitLocker drive from Daleris
- Map a network drive from the Network section's right-click menu
- A network sign-in that fails says what's wrong, with Sign in and Saved passwords buttons
- Icons Windows is slow to give arrive late instead of never

### Administration
- Policies for the sidebar's pins and order, the right-click menu and custom commands, the language, new tabs and the search box's buttons
- Policy templates moved to the PolicyDefinitions folder

## 0.0.4

- Windows Explorer integration, every part off by default and covered by policy: open folders in Daleris, Win + E, select the file on "Show in folder", new folders as tabs, Daleris for apps' Open and Save dialogs, and Ctrl + G in Windows's own dialogs. Uninstalling restores Windows's defaults
- A network path that doesn't answer never holds up the window: such folders get their own page with Cancel and Retry, and unavailable drives come back by themselves
- A Sort menu with Folders first as its own switch, a sort remembered per folder, and Downloads newest first
- A filter panel for kind, extension, date, size and name
- Dotfiles can be hidden

## 0.0.3

- Version, information and updates have their own settings page

## 0.0.2

- Updates from GitHub releases, on by default: checked at start and every 6 hours, installed the way Daleris was installed, and only when signed with Daleris's key. The AutomaticUpdates policy turns them off
- The MSI and setup.exe install for one account or the whole PC, with a setup wizard
- Icons for hundreds of file types, with outlines that tell them apart, and Daleris and Phosphor as separate icon packs
- No redrawing while idle
- Shortcuts keep their icon after an update

## 0.0.1

- The first release: tabs, list and large icon views, an editable address bar, the preview pane, folder sizes, cut, copy, paste, rename and delete
- Fast search with a query language: wildcards, exclusions, type, size, date and regular expressions
- Cloud folders in the sidebar, with Always keep on this device and Free up space on Windows
- The real Windows icons, thumbnails and right-click menu, with custom commands
- Dark and light themes, accent colours, icon packs and a rearrangeable sidebar and ribbon bar
- Group Policy templates in English and German
- Installers for Windows, Linux and macOS
