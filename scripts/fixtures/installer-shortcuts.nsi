Unicode true
!pragma warning disable 6001
!include MUI2.nsh
!include FileFunc.nsh
!include "Win\COM.nsh"
!include "Win\Propkey.nsh"
!include "${FLAIR_FIXTURE_ROOT}\utils.nsh"
!include "${FLAIR_FIXTURE_ROOT}\installer-under-test.nsh"
!define VERSION "0.7.1"
!define MAINBINARYNAME "FlairBloom"
Var UpdateMode
Name "Flair shortcut fixture"
OutFile "${FLAIR_FIXTURE_ROOT}\installer-fixture.exe"
RequestExecutionLevel user
SilentInstall silent
SilentUnInstall silent

!macro AssertFile PATH EXISTS CODE
  ${If} ${FileExists} "${PATH}"
    !if "${EXISTS}" == "false"
      SetErrorLevel ${CODE}
      Quit
    !endif
  ${Else}
    !if "${EXISTS}" == "true"
      SetErrorLevel ${CODE}
      Quit
    !endif
  ${EndIf}
!macroend

!macro AssertShortcutIcon PATH CODE
  StrCpy $3 -1
  StrCpy $2 ""
  !insertmacro ComHlpr_CreateInProcInstance ${CLSID_ShellLink} ${IID_IShellLink} r0 ""
  ${If} $0 P<> 0
    ${IUnknown::QueryInterface} $0 '("${IID_IPersistFile}", .r1)'
    ${If} $1 P<> 0
      ${IPersistFile::Load} $1 '("${PATH}", ${STGM_READ})'
      ${IShellLink::GetIconLocation} $0 '(.r2, ${NSIS_MAX_STRLEN}, .r3)'
      ${IUnknown::Release} $1 ""
    ${EndIf}
    ${IUnknown::Release} $0 ""
  ${EndIf}
  ${If} $2 != "$INSTDIR\icons\icon.ico"
  ${OrIf} $3 != 0
    SetErrorLevel ${CODE}
    Quit
  ${EndIf}
!macroend

Section
  StrCpy $INSTDIR "${FLAIR_FIXTURE_ROOT}\app"
  StrCpy $UpdateMode 0
  CreateDirectory "$INSTDIR"
  SetOutPath "$INSTDIR\icons"
  File /oname=icon.ico "${__FILEDIR__}\..\..\apps\main\src-tauri\icons\icon.ico"
  SetOutPath "$INSTDIR"
  CreateDirectory "${FLAIR_FIXTURE_ROOT}\desktop"
  ; 未选择创建快捷方式时，PostInstall 不得强制创建。
  !insertmacro NSIS_HOOK_POSTINSTALL
  !insertmacro AssertFile "${FLAIR_FIXTURE_ROOT}\desktop\气质花 v0.7.1.lnk" false 101

  CreateShortcut "${FLAIR_FIXTURE_ROOT}\desktop\FlairBloom.lnk" "$INSTDIR\FlairBloom.exe"
  CreateShortcut "${FLAIR_FIXTURE_ROOT}\desktop\气质花 v0.6.0.lnk" "$INSTDIR\FlairBloom.exe" "" "$SYSDIR\shell32.dll" 2
  CreateShortcut "${FLAIR_FIXTURE_ROOT}\desktop\气质花 v9.9.9.lnk" "${FLAIR_FIXTURE_ROOT}\other\FlairBloom.exe"
  !insertmacro NSIS_HOOK_POSTINSTALL
  !insertmacro AssertFile "${FLAIR_FIXTURE_ROOT}\desktop\气质花 v0.7.1.lnk" true 102
  !insertmacro AssertFile "${FLAIR_FIXTURE_ROOT}\desktop\FlairBloom.lnk" false 103
  !insertmacro AssertFile "${FLAIR_FIXTURE_ROOT}\desktop\气质花 v0.6.0.lnk" false 104
  !insertmacro AssertFile "${FLAIR_FIXTURE_ROOT}\desktop\气质花 v9.9.9.lnk" true 105
  !insertmacro AssertShortcutIcon "${FLAIR_FIXTURE_ROOT}\desktop\气质花 v0.7.1.lnk" 118

  ; 已是当前中文名的旧链接仍须修复错误图标。
  CreateShortcut "${FLAIR_FIXTURE_ROOT}\desktop\气质花 v0.7.1.lnk" "$INSTDIR\FlairBloom.exe" "--fixture" "$SYSDIR\shell32.dll" 2 SW_SHOWMINIMIZED "" "保留快捷方式属性"
  !insertmacro NSIS_HOOK_POSTINSTALL
  !insertmacro AssertShortcutIcon "${FLAIR_FIXTURE_ROOT}\desktop\气质花 v0.7.1.lnk" 119

  ; 模拟完成页在 PostInstall 之后创建英文链接。
  CreateShortcut "${FLAIR_FIXTURE_ROOT}\desktop\FlairBloom.lnk" "$INSTDIR\FlairBloom.exe"
  Call .onGUIEnd
  !insertmacro AssertFile "${FLAIR_FIXTURE_ROOT}\desktop\FlairBloom.lnk" false 106
  !insertmacro AssertFile "${FLAIR_FIXTURE_ROOT}\desktop\气质花 v0.7.1.lnk" true 107
  !insertmacro AssertShortcutIcon "${FLAIR_FIXTURE_ROOT}\desktop\气质花 v0.7.1.lnk" 120

  ; 全新安装的完成页迁移也应显式绑定图标。
  Delete "${FLAIR_FIXTURE_ROOT}\desktop\气质花 v0.7.1.lnk"
  CreateShortcut "${FLAIR_FIXTURE_ROOT}\desktop\FlairBloom.lnk" "$INSTDIR\FlairBloom.exe" "--fixture" "" 0 SW_SHOWMINIMIZED "" "保留快捷方式属性"
  Call .onGUIEnd
  !insertmacro AssertShortcutIcon "${FLAIR_FIXTURE_ROOT}\desktop\气质花 v0.7.1.lnk" 121

  ; 模拟下一版更新，只迁移原有链接，不另建重复项。
  StrCpy $FlairDesktopShortcutName "气质花 v0.7.2.lnk"
  Call FlairNormalizeDesktopShortcut
  !insertmacro AssertFile "${FLAIR_FIXTURE_ROOT}\desktop\气质花 v0.7.2.lnk" true 108
  !insertmacro AssertFile "${FLAIR_FIXTURE_ROOT}\desktop\气质花 v0.7.1.lnk" false 109
  !insertmacro AssertShortcutIcon "${FLAIR_FIXTURE_ROOT}\desktop\气质花 v0.7.2.lnk" 122

  ; 目标名称被另一份安装占用时，不覆盖它，也不丢弃本安装的原链接。
  CreateShortcut "${FLAIR_FIXTURE_ROOT}\desktop\气质花 v0.7.3.lnk" "${FLAIR_FIXTURE_ROOT}\other\FlairBloom.exe" "" "$SYSDIR\shell32.dll" 7
  CreateShortcut "${FLAIR_FIXTURE_ROOT}\desktop\FlairBloom.lnk" "$INSTDIR\FlairBloom.exe"
  StrCpy $FlairDesktopShortcutName "气质花 v0.7.3.lnk"
  Call FlairNormalizeDesktopShortcut
  !insertmacro AssertFile "${FLAIR_FIXTURE_ROOT}\desktop\FlairBloom.lnk" true 110
  !insertmacro AssertFile "${FLAIR_FIXTURE_ROOT}\desktop\气质花 v0.7.2.lnk" true 111
  !insertmacro AssertFile "${FLAIR_FIXTURE_ROOT}\desktop\气质花 v0.7.3.lnk" true 112
  WriteUninstaller "$INSTDIR\uninstall-fixture.exe"
SectionEnd

Section Uninstall
  StrCpy $INSTDIR "${FLAIR_FIXTURE_ROOT}\app"
  StrCpy $UpdateMode 1
  !insertmacro NSIS_HOOK_POSTUNINSTALL
  !insertmacro AssertFile "${FLAIR_FIXTURE_ROOT}\desktop\气质花 v0.7.2.lnk" true 113
  StrCpy $UpdateMode 0
  !insertmacro NSIS_HOOK_POSTUNINSTALL
  !insertmacro AssertFile "${FLAIR_FIXTURE_ROOT}\desktop\FlairBloom.lnk" false 114
  !insertmacro AssertFile "${FLAIR_FIXTURE_ROOT}\desktop\气质花 v0.7.2.lnk" false 115
  !insertmacro AssertFile "${FLAIR_FIXTURE_ROOT}\desktop\气质花 v0.7.3.lnk" true 116
  !insertmacro AssertFile "${FLAIR_FIXTURE_ROOT}\desktop\气质花 v9.9.9.lnk" true 117
SectionEnd
