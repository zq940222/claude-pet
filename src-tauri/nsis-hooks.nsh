; NSIS 安装钩子。由 tauri.conf.json 的 bundle.windows.nsis.installerHooks 引入。
;
; 处理和 install.ps1 的位置冲突 —— 三个位置各不相同，而自启只有一个值名：
;
;   install.ps1  ->  %LOCALAPPDATA%\ClaudePet\claude-pet.exe
;   本安装包     ->  %LOCALAPPDATA%\Claude Pet\claude-pet.exe   ($INSTDIR)
;   自启         ->  HKCU\...\Run\Claude Pet   （只有这一个）
;
; 先用 install.ps1 装过、再跑安装包的人会留下两份 exe。自启指向后装的那份，
; 另一份成了永远不会被更新、也没人会想起来删的孤儿。
;
; 这个文件被 !include 在所有 !define 之前，但宏体是在**插入点**展开的，
; 所以 ${MAINBINARYNAME} / ${PRODUCTNAME} 在这里可以正常引用。
;
; 刻意**不动** %APPDATA%\com.opsmateai.claude-pet\ 里的 prefs.json /
; sessions.json / window-anchor.json —— 那是用户的设置，换个安装方式不该
; 把它们清零。卸载时也一样保留，见 README。

!macro NSIS_HOOK_PREINSTALL
  DetailPrint "检查 install.ps1 留下的旧副本..."

  ; 先把在跑的挂件结束掉，否则删文件会失败。
  ; 不能只杀新位置的 —— 现在跑着的很可能正是旧位置那份。
  nsExec::Exec 'taskkill /IM ${MAINBINARYNAME}.exe /F'
  Pop $0

  StrCpy $R0 "$LOCALAPPDATA\ClaudePet"

  ${If} ${FileExists} "$R0\${MAINBINARYNAME}.exe"
    ; 自启值必须跟着重指，否则会留下最糟的一种状态：
    ; auto-launch 的 is_enabled() 只检查这个值**存在**、不比对路径
    ; （auto-launch-0.5.0/src/windows.rs 的 get_value(...).is_ok()），
    ; 所以删掉旧 exe 而不改值 = 设置窗口显示「已开启」但开机根本拉不起来。
    ;
    ; 只在它确实指向旧目录时才改，别覆盖用户手工设成别的东西的值。
    ; 写的格式要和 auto-launch 的 enable() 一致："{app_path} {args}"，
    ; 无参数时就是路径后面跟一个空格。
    ReadRegStr $R1 HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "${PRODUCTNAME}"
    ${If} $R1 != ""
      ${StrLoc} $R2 "$R1" "$R0" ">"
      ${If} $R2 != ""
        DetailPrint "把开机自启重新指向新位置"
        WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Run" \
          "${PRODUCTNAME}" "$INSTDIR\${MAINBINARYNAME}.exe "
      ${EndIf}
    ${EndIf}

    DetailPrint "移除 $R0"
    RMDir /r "$R0"
  ${EndIf}
!macroend
