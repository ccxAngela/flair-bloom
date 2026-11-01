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

Section
  StrCpy $INSTDIR "${FLAIR_FIXTURE_ROOT}\app"
  StrCpy $UpdateMode 0
  CreateDirectory "$INSTDIR"
  CreateDirectory "${FLAIR_FIXTURE_ROOT}\desktop"
  ; 未选择创建快捷方式时，PostInstall 不得强制创建。
  !insertmacro NSIS_HOOK_POSTINSTALL
  !insertmacro AssertFile "${FLAIR_FIXTURE_ROOT}\desktop\气质花 v0.7.1.lnk" false 101

  CreateShortcut "${FLAIR_FIXTURE_ROOT}\desktop\FlairBloom.lnk" "$INSTDIR\FlairBloom.exe"
  CreateShortcut "${FLAIR_FIXTURE_ROOT}\desktop\气质花 v0.6.0.lnk" "$INSTDIR\FlairBloom.exe"
  CreateShortcut "${FLAIR_FIXTURE_ROOT}\desktop\气质花 v9.9.9.lnk" "${FLAIR_FIXTURE_ROOT}\other\FlairBloom.exe"
  !insertmacro NSIS_HOOK_POSTINSTALL
  !insertmacro AssertFile "${FLAIR_FIXTURE_ROOT}\desktop\气质花 v0.7.1.lnk" true 102
  !insertmacro AssertFile "${FLAIR_FIXTURE_ROOT}\desktop\FlairBloom.lnk" false 103
  !insertmacro AssertFile "${FLAIR_FIXTURE_ROOT}\desktop\气质花 v0.6.0.lnk" false 104
  !insertmacro AssertFile "${FLAIR_FIXTURE_ROOT}\desktop\气质花 v9.9.9.lnk" true 105

  ; 模拟完成页在 PostInstall 之后创建英文链接。
  CreateShortcut "${FLAIR_FIXTURE_ROOT}\desktop\FlairBloom.lnk" "$INSTDIR\FlairBloom.exe"
  Call .onGUIEnd
  !insertmacro AssertFile "${FLAIR_FIXTURE_ROOT}\desktop\FlairBloom.lnk" false 106
  !insertmacro AssertFile "${FLAIR_FIXTURE_ROOT}\desktop\气质花 v0.7.1.lnk" true 107

  ; 模拟下一版更新，只迁移原有链接，不另建重复项。
  StrCpy $FlairDesktopShortcutName "气质花 v0.7.2.lnk"
  Call FlairNormalizeDesktopShortcut
  !insertmacro AssertFile "${FLAIR_FIXTURE_ROOT}\desktop\气质花 v0.7.2.lnk" true 108
  !insertmacro AssertFile "${FLAIR_FIXTURE_ROOT}\desktop\气质花 v0.7.1.lnk" false 109

  ; 目标名称被另一份安装占用时，不覆盖它，也不丢弃本安装的原链接。
  CreateShortcut "${FLAIR_FIXTURE_ROOT}\desktop\气质花 v0.7.3.lnk" "${FLAIR_FIXTURE_ROOT}\other\FlairBloom.exe"
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
