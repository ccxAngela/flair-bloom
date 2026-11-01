; FlairBloom NSIS installer hooks
;
; interception.dll 通过 bundle.resources 打入安装包，落地在 $INSTDIR\resources\，
; 但 EXE 隐式链接 interception.dll 需要它与 EXE 同级（或在系统 DLL 搜索路径上）。
; PostInstall 阶段把它移动到 $INSTDIR\，卸载时一并清理。

Var FlairDesktopShortcutName
Var FlairDesktopShortcutTarget
Var FlairDesktopShortcutIconChanged

; 保留参数、工作目录及 Shell 属性，只修正图标并定向刷新资源管理器缓存。
!macro FLAIR_REFRESH_SHORTCUT_ICON SHORTCUT
  !insertmacro ComHlpr_CreateInProcInstance ${CLSID_ShellLink} ${IID_IShellLink} r0 ""
  ${If} $0 P<> 0
    ${IUnknown::QueryInterface} $0 '("${IID_IPersistFile}", .r1)'
    ${If} $1 P<> 0
      ${IPersistFile::Load} $1 '("${SHORTCUT}", ${STGM_READWRITE}).r2'
      ${If} $2 = 0
        ${IShellLink::SetIconLocation} $0 '("$INSTDIR\icons\icon.ico", 0).r2'
        ${If} $2 = 0
          ${IPersistFile::Save} $1 '("${SHORTCUT}", 1).r2'
          ${If} $2 = 0
            StrCpy $FlairDesktopShortcutIconChanged 1
            ; SHCNE_UPDATEITEM / SHCNF_PATHW | SHCNF_FLUSHNOWAIT
            System::Call 'shell32::SHChangeNotify(i 0x2000, i 0x2005, w "${SHORTCUT}", p 0)'
          ${EndIf}
        ${EndIf}
      ${EndIf}
      ${IUnknown::Release} $1 ""
    ${EndIf}
    ${IUnknown::Release} $0 ""
  ${EndIf}
!macroend

; 同名链接可能属于另一份安装，只处理目标确认为本安装目录的链接。
!macro FLAIR_DESKTOP_SHORTCUTS REMOVE
  Push $0
  Push $1
  Push $2
  Push $3
  Push $R0
  Push $R1
  ClearErrors
  FindFirst $R0 $R1 "$DESKTOP\*.lnk"
  ${IfNot} ${Errors}
    ${DoWhile} $R1 != ""
      StrCpy $0 $R1 5
      ${If} $R1 == "FlairBloom.lnk"
      ${OrIf} $R1 == "气质花.lnk"
      ${OrIf} $0 == "气质花 v"
        !insertmacro IsShortcutTarget "$DESKTOP\$R1" "$FlairDesktopShortcutTarget"
        Pop $0
        ${If} $0 = 1
          !if "${REMOVE}" == "true"
            Delete "$DESKTOP\$R1"
          !else
            !insertmacro FLAIR_REFRESH_SHORTCUT_ICON "$DESKTOP\$R1"
            ${If} $R1 != $FlairDesktopShortcutName
              ${If} ${FileExists} "$DESKTOP\$FlairDesktopShortcutName"
                !insertmacro IsShortcutTarget "$DESKTOP\$FlairDesktopShortcutName" "$FlairDesktopShortcutTarget"
                Pop $0
                ${If} $0 = 1
                  Delete "$DESKTOP\$R1"
                ${EndIf}
              ${Else}
                ClearErrors
                Rename "$DESKTOP\$R1" "$DESKTOP\$FlairDesktopShortcutName"
                ${IfNot} ${Errors}
                  ; SHCNE_RENAMEITEM：通知包含中文的新路径，避免桌面沿用改名前的图标。
                  System::Call 'shell32::SHChangeNotify(i 1, i 0x2005, w "$DESKTOP\$R1", w "$DESKTOP\$FlairDesktopShortcutName")'
                ${EndIf}
              ${EndIf}
            ${EndIf}
          !endif
        ${EndIf}
      ${EndIf}
      FindNext $R0 $R1
    ${Loop}
    FindClose $R0
  ${EndIf}
  Pop $R1
  Pop $R0
  Pop $3
  Pop $2
  Pop $1
  Pop $0
  ClearErrors
!macroend

Function FlairNormalizeDesktopShortcut
  StrCpy $FlairDesktopShortcutIconChanged 0
  !insertmacro FLAIR_DESKTOP_SHORTCUTS false
  ${If} $FlairDesktopShortcutIconChanged = 1
    ; Explorer 可能继续显示白纸图标，单条路径通知不足以使其进程内缓存失效。
    ; 每轮整理后仅刷新一次；没有本安装的桌面链接时不触发。
    System::Call 'shell32::SHChangeNotify(i 0x08000000, i 0x1000, p 0, p 0)'
  ${EndIf}
FunctionEnd

Function un.FlairRemoveDesktopShortcuts
  !insertmacro FLAIR_DESKTOP_SHORTCUTS true
FunctionEnd

; 交互安装在完成页才按用户选择创建快捷方式，不能只在 PostInstall 中改名。
Function .onGUIEnd
  ${If} $FlairDesktopShortcutName != ""
    Call FlairNormalizeDesktopShortcut
  ${EndIf}
FunctionEnd

!macro NSIS_HOOK_POSTINSTALL
  ${If} ${FileExists} "$INSTDIR\resources\interception.dll"
    Rename "$INSTDIR\resources\interception.dll" "$INSTDIR\interception.dll"
  ${EndIf}
  StrCpy $FlairDesktopShortcutName "气质花 v${VERSION}.lnk"
  StrCpy $FlairDesktopShortcutTarget "$INSTDIR\${MAINBINARYNAME}.exe"
  Call FlairNormalizeDesktopShortcut
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
  ; 更新时保留旧链接，让新安装器迁移版本号；真正卸载才删除。
  ${If} $UpdateMode <> 1
    StrCpy $FlairDesktopShortcutTarget "$INSTDIR\${MAINBINARYNAME}.exe"
    Call un.FlairRemoveDesktopShortcuts
  ${EndIf}
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  Delete "$INSTDIR\interception.dll"
  ; 「以管理员模式启动」写在当前用户的兼容性标志里，卸载时清掉，
  ; 免得注册表留一条指向已不存在的 EXE 的记录。
  DeleteRegValue HKCU "Software\Microsoft\Windows NT\CurrentVersion\AppCompatFlags\Layers" "$INSTDIR\FlairBloom.exe"
!macroend
