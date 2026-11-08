Daleris: policies for administrators

Windows uses Group Policy (GPO and GPP); Linux and macOS use a file, see the end of this guide.
Daleris reads its policies when it starts and every 30 seconds while it runs, so a change
reaches open windows without a restart.

FIXED SETTINGS (people cannot change them, the app grays them out with a lock)
  HKLM\SOFTWARE\Policies\Daleris       checked first
  HKCU\SOFTWARE\Policies\Daleris       used for anything the machine key leaves open

DEFAULT SETTINGS (apply until a person picks their own value, and can always be changed)
  HKLM\SOFTWARE\Policies\Daleris\Recommended
  HKCU\SOFTWARE\Policies\Daleris\Recommended
  HKLM\SOFTWARE\Daleris\Defaults
  HKCU\SOFTWARE\Daleris\Defaults
  The first one holding a value wins. A person's own choice stays when a default changes later.

USING GPO
  1. Copy Daleris.admx to \\<domain>\SYSVOL\<domain>\Policies\PolicyDefinitions
     (or C:\Windows\PolicyDefinitions on a single machine).
  2. Copy the en-US and de-DE folders next to it, keeping their Daleris.adml files inside.
  3. Find "Daleris" under Administrative Templates, for computers and for users.
     "Fixed settings" and "Default settings" hold the same list.

USING GPP (Group Policy Preferences)
  Add Registry items under Preferences > Windows Settings > Registry, using the keys and value
  names here. This lets you use item-level targeting (by group, site, OS and so on).
  For a fixed setting write to SOFTWARE\Policies\Daleris. For a default that only the
  first deployment should set, write to SOFTWARE\Daleris\Defaults with the action "Create".
  Values can be REG_DWORD or REG_SZ, and the search lists REG_MULTI_SZ, one item per line. Preferences stay in the registry after the item is removed
  unless you tick "Remove this item when it is no longer applied".

VALUES
  Name                      Type      Meaning
  ShowHiddenItems           DWORD     0 off, 1 on
  HideDotfiles              DWORD     0 off, 1 hides names that start with a dot. An item is hidden if this or ShowHiddenItems hides it.
                                      Windows only: on Linux and macOS those names are hidden items, see ShowHiddenItems
  ExplorerLook              DWORD     1 sets ExplorerColours, ExplorerLines, ExplorerMica and ExplorerMicaBar on, IconPack
                                      to system and RowHeight to 0; 0 sets them back to Daleris's own. Fixed, it fixes them
                                      all. Windows only, where it's on by default. TitleBarLogo and TitleBarName stay apart
  ExplorerColours           DWORD     1 on, 0 off: Explorer's colours, following Windows's light or dark mode and accent colour.
                                      Theme and AccentColor don't apply then. Windows only
  ExplorerLines             DWORD     1 on, 0 off: Explorer's lines between the panes and around the tab in front, and the
                                      open place in the sidebar marked by its colour alone. Windows only
  ExplorerMica              DWORD     1 on, 0 off: Mica behind the tab bar. Windows 11 (22H2 and later); takes effect
                                      when Daleris starts
  ExplorerMicaBar           DWORD     1 on, 0 off: with ExplorerMica, a little of it through the tab in front and the
                                      address bar, as in Explorer. Windows 11 (22H2 and later)
  OpenNewDrives             DWORD     1 on, 0 off: a tab opens when Windows opens a drive that was just connected, such
                                      as a USB stick. Windows only
  StartNewWindow            DWORD     1 on, 0 off: starting Daleris again, from Start, the taskbar or Win+E, opens a new
                                      window; off, a new tab in the open one. Windows only
  ShareTransferNotes        DWORD     1 on, 0 off: while copying, moving or deleting on a network share, or with files open
                                      there, Daleris leaves a small hidden note in the folder (.daleris-transit-*), with the
                                      person's name and Windows account picture, so Daleris on other PCs shows it, and removes it
                                      when done
  DefaultSort               SZ        name, modified, created, type, extension or size: how a folder is sorted until someone chooses
  DefaultSortAscending      DWORD     1 from A, the oldest or the smallest, 0 the other way round
  FoldersFirst              DWORD     1 keeps folders above files by default, 0 mixes them in
  DownloadsNewestFirst      DWORD     1 opens the Downloads folder newest first with folders mixed in, 0 treats it like any other
  ShowPreviewPane           DWORD     0 off, 1 on
  ShowFileExtensions        DWORD     0 off, 1 on
  DefaultView               DWORD     0 details, 1 large icons
  Theme                     DWORD     0 follow Windows, 1 dark, 2 light
  AccentColor               DWORD     7 Windows accent colour, 0 to 6 the colours in Settings, left to right
  IconPack                  SZ        system (Windows icons, Windows only), flat (Daleris), phosphor, or the name of a
                                      folder in %APPDATA%\Daleris\icon packs
  RowHeight                 DWORD     0 compact, 1 normal, 2 comfortable
  CustomTitleBar            DWORD     0 standard Windows title bar, 1 Daleris title bar
  TitleBarLogo              DWORD     0 off, 1 on
  TitleBarName              DWORD     0 off, 1 on
  TitleBarSettingsButton    DWORD     0 off, 1 on
  TitleBarBrandSettings     DWORD     1 on, 0 off: with TitleBarLogo or TitleBarName, they open Settings and the settings
                                      button isn't shown
  ReopenTabs                DWORD     0 off, 1 on
  NewTabOpens               SZ        home (the default), same (the folder of the tab in front), thispc, or a folder's
                                      path: where a new tab opens. With Shift, it's always the folder of the tab in front
  WindowsPropertiesDialog   DWORD     0 Daleris's, 1 the Windows dialog
  FolderSizes               DWORD     0 off, 1 on
  SearchSubfolders          DWORD     0 off, 1 on
  SearchRegex               DWORD     0 off, 1 on
  SearchSkipAppData         DWORD     1: search never finds folders named AppData
  SearchSkipWindows         DWORD     1: search never finds folders named Windows, Program Files, Program Files (x86),
                                      ProgramData, $Recycle.Bin, System Volume Information, $WinREAgent or Recovery
  SearchSkipDeveloperFiles  DWORD     1: search never finds folders named node_modules, .git, .svn, target, __pycache__,
                                      .cache, .cargo, .rustup, .npm, .gradle, .venv or venv
  SearchSkipCloudFolders    DWORD     1: search never finds folders named OneDrive, Dropbox, Google Drive or iCloudDrive
  SearchSkipTempFiles       DWORD     1: search never finds .tmp, .temp, .log, .bak or .dmp files
  SearchSkipProgramParts    DWORD     1: search never finds .dll, .sys, .mui, .cat, .pdb, .obj or .lib files
  SearchSkipHidden          DWORD     1: search never finds hidden items, even when hidden items are shown
  SearchSkipSystem          DWORD     1: search never finds what Windows marks as system files, such as desktop.ini.
                                      On unless set to 0. Windows only
  SearchSkipFolders         MULTI_SZ  folders search never finds, with everything in them. %USERPROFILE% and other
                                      environment variables work, and on Linux and macOS ~
  SearchSkipFolderNames     MULTI_SZ  folder names search never finds, wherever they are
  SearchSkipFileTypes       MULTI_SZ  file types search never finds, written .iso, iso or *.iso
  SearchSkipPatterns        MULTI_SZ  regular expressions: search never finds a path that matches one. See SEARCH below.
                                      The four lists take one item per line. All but SearchSkipPatterns can also be
                                      REG_SZ with items separated by |; for SearchSkipPatterns a REG_SZ holds one pattern
  AutomaticUpdates          DWORD     0 off, 1 on. Off, people can still look for an update and install it from Settings
  RibbonLeft                SZ        ribbon items on the left, in order, separated by |
  RibbonRight               SZ        ribbon items on the right, in order, separated by |
  RibbonHidden              SZ        ribbon items that are switched off, separated by |
  SidebarHiddenSections     SZ        any of quick, cloud, pc, network, separated by |
                                      (as a default this only applies the first time the app runs)
  SidebarPins               SZ        folders pinned to the sidebar, separated by |. section>folder pins to a
                                      section (quick, cloud, pc, network), a plain folder to Quick access.
                                      %USERPROFILE% and other environment variables work, and on Linux and macOS ~
                                      (as a default this only applies the first time the app runs)
  SidebarOrder              SZ        sections in order, any of quick, cloud, pc, network, separated by |
                                      (as a default this only applies the first time the app runs)
  ContextMenuHidden         SZ        right-click menu items never shown, separated by |. See MENU ITEMS below
  ContextMenuMoreOptions    SZ        right-click menu items moved into Show more options, separated by |
  ContextMenuOrder          SZ        right-click menu items in order, separated by |. Items left out follow
  CustomCommands            SZ        commands in the right-click menu, separated by |. Each is name>command or
                                      name>items>command, items being all, files, folders, background or
                                      ext:jpg,png. {path}, {paths}, {name} and {dir} work in the command.
                                      The command can't hold | or >. As a fixed value it replaces people's own
                                      commands, which come back without it
                                      (as a default this only applies the first time the app runs)

  ExplorerOpenFolders       DWORD     0 off, 1 on. Folders from the desktop, Start and most apps open in Daleris
  ExplorerWinE              DWORD     0 off, 1 on. Win + E opens Daleris, through the setting Windows reads for it
  ExplorerWinEHook          DWORD     0 off, 1 on. Win + E through a keyboard hook in a small background program
  ExplorerSelectFile        DWORD     0 off, 1 on. "Show in folder" selects the file (needs ExplorerOpenFolders)
  ExplorerSingleWindow      DWORD     0 off, 1 on. New folders become tabs in the running window
  ExplorerFileDialogs       DWORD     0 off, 1 on. Open and Save dialogs use Daleris (loads a DLL into apps, see below)
  ExplorerDialogJump        DWORD     0 off, 1 on. Ctrl + G takes a Windows file dialog to the Daleris folder
  ExplorerDialogLog         DWORD     0 off, 1 on. The DLL writes what it decides to %LOCALAPPDATA%\Daleris\logs\file-dialogs.log
  ExplorerDialogExclusions  SZ        apps that keep the Windows file dialog, separated by |, as names or folders.
                                      !name.exe lets Daleris show a browser's dialogs

  Ribbon items: new sep1 cut copy paste rename delete sep2 sort filter view more terminal hidden preview
  Fixing RibbonLeft or RibbonRight shows only the items listed on either side.

LOCKS (under the fixed keys only: DWORD, 1 turns it on)
  LockRibbon                The ribbon bar uses the policy values and built-in layout, and cannot be customized.
  LockSidebar               The sidebar cannot be rearranged, hidden or pinned to. Sections can still open and close.
  LockContextMenu           The right-click menu cannot be customized.
  DisableCustomCommands     Commands people added are not shown or run, and none can be added.
  DisableUpdates            Daleris does not look for updates or install them, not even from Settings.

UPDATES ON A PC-WIDE INSTALL
  A task, "Daleris Updater", installs them as SYSTEM, which can't see anyone's user policies or settings.
  So for those installs, set AutomaticUpdates and DisableUpdates under Computer Configuration (HKLM).
  The task goes by, in order: DisableUpdates, a fixed AutomaticUpdates, the switch in Settings (which is
  kept in %ProgramData%\Daleris\auto-update.txt for the whole PC), a default AutomaticUpdates. Otherwise it's on.
  A fixed AutomaticUpdates of 1 means no user of the PC can turn updates off.

MENU ITEMS
  ContextMenuHidden, ContextMenuMoreOptions and ContextMenuOrder name items like this:
    Daleris's own   app:newtab, app:newwindow, app:terminal, app:openwith, app:runadmin, app:location,
                    app:linktarget, app:copycontents, app:pin, app:new, app:newtext, app:paste, app:sort,
                    app:refresh, app:selectall, app:deleteperm, app:disconnect, app:cloudkeep, app:cloudfree,
                    app:properties, app:moresub (Show more options), app:more (the classic Windows menu),
                    app:customize
    Windows'        v: and the verb, such as v:open, v:edit or v:print
                    or l: and the label in lower case, such as l:scan with microsoft defender
  Items people move under Settings > Right-click menu are written to the settings file as menu= lines, which show
  the name to use for any item. A fixed hidden or more-options list sets the whole menu: items it doesn't name go
  where Daleris puts them, and the menu can't be customized. Any of the three as a default applies to each item
  until the person moves it.

SEARCH
  What search never finds isn't a result, and search doesn't look inside a folder it leaves out. Browsing still
  shows everything. People tick the same groups and fill the same lists under Settings > Files & folders >
  Leave out of search.
  A fixed list is added to people's own rules: they can't remove its items, but they can still add their own.
  A default list is the list people start with. The group switches work like any other value and cover the whole
  group: people can tick a group's items one by one, but a fixed group is all on or all off. To leave out only
  part of a group for everyone, put those names in SearchSkipFolderNames or SearchSkipFileTypes instead.
  Patterns are matched against the full path, whatever the case. A folder's path ends in \ when it's matched, so
  \\Payroll\\ leaves out every Payroll folder with everything in it, and \.(key|pem|pfx)$ leaves out keys and
  certificates. A pattern that isn't valid is ignored, and Settings shows why.
  System files are left out unless SearchSkipSystem is 0.

WINDOWS EXPLORER INTEGRATION
  The Explorer values above only mean something on Windows, and are ignored on Linux and macOS.
  Everything they change is in each person's own HKCU, so they work per user, never per machine, whichever part of
  Group Policy sets them.
  Fixed to 0: what the switch wrote is removed when Daleris next runs for that person, or within 30 seconds if it's
  running. Until then the launcher and the DLL read the same policy and step aside by themselves: a folder opens in File
  Explorer and an app gets the Windows dialog, straight away.
  Fixed to 1, or as a default: it can only be registered from that person's own session, so it takes effect once Daleris
  has run for them. Nothing is set up for someone who has never started Daleris.
  ExplorerFileDialogs loads an unsigned DLL into apps, when they open a dialog. Games, launchers, anti-cheat and security
  software are always left alone, as are elevated, sandboxed and Microsoft Store apps. Check it with your antivirus or
  application control policy before deploying it. It's not offered on ARM PCs.
  daleris --restore-explorer removes everything Daleris wrote for the person who runs it, and nothing else.

Each person's own choice is kept underneath a fixed setting and comes back if the policy is removed.

LINUX AND MACOS
  Linux:  /etc/daleris/policy.conf
  macOS:  /Library/Application Support/Daleris/policy.conf

  Make it writable only by root. It uses the names above, one per line, as text. Values before any
  heading, or after [Fixed], are fixed. Those after [Recommended] are defaults:

    # a comment
    AutomaticUpdates=0
    DisableCustomCommands=1
    [Recommended]
    Theme=2
    IconPack=phosphor

  The search lists take a line per item, repeating the name:

    SearchSkipPatterns=/Payroll/
    SearchSkipPatterns=\.(key|pem|pfx)$
