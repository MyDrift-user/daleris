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
  Values can be REG_DWORD or REG_SZ. Preferences stay in the registry after the item is removed
  unless you tick "Remove this item when it is no longer applied".

VALUES
  Name                      Type    Meaning
  ShowHiddenItems           DWORD   0 off, 1 on
  ShowPreviewPane           DWORD   0 off, 1 on
  ShowFileExtensions        DWORD   0 off, 1 on
  DefaultView               DWORD   0 details, 1 large icons
  Theme                     DWORD   0 follow Windows, 1 dark, 2 light
  AccentColor               DWORD   7 Windows accent colour, 0 to 6 the colours in Settings, left to right
  IconPack                  SZ      system (Windows icons, Windows only), flat (Daleris), phosphor, or the name of a
                                    folder in %APPDATA%\Daleris\icon packs
  RowHeight                 DWORD   0 compact, 1 normal, 2 comfortable
  CustomTitleBar            DWORD   0 standard Windows title bar, 1 Daleris title bar
  TitleBarLogo              DWORD   0 off, 1 on
  TitleBarName              DWORD   0 off, 1 on
  TitleBarSettingsButton    DWORD   0 off, 1 on
  ReopenTabs                DWORD   0 off, 1 on
  WindowsPropertiesDialog   DWORD   0 Daleris's, 1 the Windows dialog
  FolderSizes               DWORD   0 off, 1 on
  SearchSubfolders          DWORD   0 off, 1 on
  SearchRegex               DWORD   0 off, 1 on
  AutomaticUpdates          DWORD   0 off, 1 on. Off, people can still look for an update and install it from Settings
  RibbonLeft                SZ      ribbon items on the left, in order, separated by |
  RibbonRight               SZ      ribbon items on the right, in order, separated by |
  RibbonHidden              SZ      ribbon items that are switched off, separated by |
  SidebarHiddenSections     SZ      any of quick, cloud, pc, network, separated by |
                                    (as a default this only applies the first time the app runs)

  Ribbon items: new sep1 cut copy paste rename delete sep2 sort view more terminal hidden preview
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
