; NSIS 安装钩子。由 tauri.conf.json 的 bundle.windows.nsis.installerHooks 引入。
;
; 只做一件事：清掉 install.ps1 留下的旧副本。
;
; 为什么必须做 —— 三个位置各不相同，而自启只有一个值名：
;
;   install.ps1  ->  %LOCALAPPDATA%\ClaudePet\claude-pet.exe
;   本安装包     ->  %LOCALAPPDATA%\Claude Pet\claude-pet.exe
;   自启         ->  HKCU\...\Run\Claude Pet   （只有这一个）
;
; 先用 install.ps1 装过、再跑安装包的人会留下两份 exe。自启指向后装的那份，
; 另一份成了永远不会被更新、也没人会想起来删的孤儿。本机实测就是这个状态。
;
; 刻意**不动** %APPDATA%\com.opsmateai.claude-pet\ 里的 prefs.json /
; sessions.json / window-anchor.json —— 那是用户的设置，换个安装方式不该
; 把它们清零。卸载时也一样保留，见 README。

!macro NSIS_HOOK_PREINSTALL
  DetailPrint "检查旧版本安装（install.ps1 的位置）..."

  ; 先把在跑的挂件结束掉，否则删文件会失败。
  ; 这里不能只杀新位置的 —— 现在跑着的很可能正是旧位置那份。
  nsExec::Exec 'taskkill /IM claude-pet.exe /F'
  Pop $0

  ${If} ${FileExists} "$LOCALAPPDATA\ClaudePet\claude-pet.exe"
    DetailPrint "移除 $LOCALAPPDATA\ClaudePet"
    ; 只在确认里面就是我们的 exe 时才递归删，避免误伤同名目录
    RMDir /r "$LOCALAPPDATA\ClaudePet"
  ${EndIf}
!macroend
