; Points d'ancrage du wizard NSIS (Tauri v2) — référencés par
; tauri.conf.json > bundle > windows > nsis > installerHooks.
;
; Épinglage barre des tâches (Win10/11) : le verbe shell « taskbarpin » copie
; le raccourci du menu Démarrer dans User Pinned\TaskBar — l'épingle apparaît
; immédiatement, sans redémarrage d'Explorer. Le désinstallateur désépingle
; AVANT de supprimer les fichiers : le verbe a besoin que le .lnk source
; existe encore. ExecShell n'est jamais fatale : un verbe refusé par une
; stratégie de parquet ne bloque pas l'installation.

!macro NSIS_HOOK_PREINSTALL
  ; MC-C03 : en mode silencieux (/S, auto-mise à jour), laisser le temps au
  ; processus parent qui vient d'appeler exit(0) de libérer son handle binaire.
  IfSilent 0 +2
    Sleep 600
!macroend

!macro NSIS_HOOK_POSTINSTALL
  ; Épingler le raccourci du menu Démarrer à la barre des tâches.
  ExecShell "taskbarpin" "$SMPROGRAMS\$AppStartMenuFolder\${PRODUCTNAME}.lnk"
  ; En mode silencieux (/S, déclenché par l'auto-mise à jour depuis MCP Control),
  ; relancer automatiquement l'application sitôt l'installation terminée.
  IfSilent 0 +2
    Exec '"$INSTDIR\${MAINBINARYNAME}.exe"'
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  ; Désépingler avant la suppression du raccourci.
  ExecShell "taskbarunpin" "$SMPROGRAMS\$AppStartMenuFolder\${PRODUCTNAME}.lnk"
!macroend
