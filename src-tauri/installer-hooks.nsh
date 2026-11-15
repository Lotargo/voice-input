; The stock Tauri uninstaller removes APPDATA/LOCALAPPDATA only when this flag is 1.
; Upgrades and unattended uninstalls retain all data. Normal uninstall asks explicitly.
!macro NSIS_HOOK_PREUNINSTALL
  ${If} $UpdateMode = 1
    StrCpy $DeleteAppDataCheckboxState 0
  ${Else}
    IfSilent voice_keep_data
    MessageBox MB_YESNO|MB_ICONQUESTION|MB_DEFBUTTON1 "Сохранить скачанные модели, незавершённые загрузки и настройки Voice Input?$\r$\n$\r$\nДа - сохранить для следующей установки или portable-версии.$\r$\nНет - удалить общие данные всех копий Voice Input этого пользователя." IDYES voice_keep_data IDNO voice_remove_data
    voice_remove_data:
      StrCpy $DeleteAppDataCheckboxState 1
      Goto voice_data_choice_done
    voice_keep_data:
      StrCpy $DeleteAppDataCheckboxState 0
    voice_data_choice_done:
  ${EndIf}
!macroend
