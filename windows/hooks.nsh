!macro BDO_EXPECTED_INSTALL_DIR
  !if "${ARCH}" == "x64"
    StrCpy $R8 "$PROGRAMFILES64\${PRODUCTNAME}"
  !else if "${ARCH}" == "arm64"
    StrCpy $R8 "$PROGRAMFILES64\${PRODUCTNAME}"
  !else
    StrCpy $R8 "$PROGRAMFILES\${PRODUCTNAME}"
  !endif
!macroend

!macro BDO_REJECT_REPARSE_INSTALL_DIR
  Push $9
  System::Call 'kernel32::GetFileAttributesW(w "$INSTDIR") i .r9'
  ${If} $9 != -1
    IntOp $9 $9 & 0x00000400 ; FILE_ATTRIBUTE_REPARSE_POINT
    ${If} $9 <> 0
      Abort "BDO Optimizer 설치 경로가 reparse point이므로 설치를 중단합니다."
    ${EndIf}
  ${EndIf}
  Pop $9
!macroend

Function BDO_RejectReparseTree
  Exch $R0
  Push $R1
  Push $R2
  Push $R3
  Push $4
  Push $5

  ClearErrors
  FindFirst $R1 $R2 "$R0\*"
  ${If} ${Errors}
    Abort "BDO Optimizer 설치 트리 열거에 실패했습니다: $R0"
  ${EndIf}

  bdo_reparse_tree_loop:
    StrCmp $R2 "" bdo_reparse_tree_done
    StrCmp $R2 "." bdo_reparse_tree_next
    StrCmp $R2 ".." bdo_reparse_tree_next
    StrCpy $R3 "$R0\$R2"
    System::Call 'kernel32::GetFileAttributesW(w "$R3") i .r4'
    ${If} $4 = -1
      FindClose $R1
      Abort "BDO Optimizer 설치 트리 속성 조회에 실패했습니다: $R3"
    ${EndIf}
    IntOp $5 $4 & 0x00000400 ; FILE_ATTRIBUTE_REPARSE_POINT
    ${If} $5 <> 0
      FindClose $R1
      Abort "BDO Optimizer 설치 트리에 reparse point가 있어 설치를 중단합니다: $R3"
    ${EndIf}
    IntOp $5 $4 & 0x00000010 ; FILE_ATTRIBUTE_DIRECTORY
    ${If} $5 <> 0
      Push "$R3"
      Call BDO_RejectReparseTree
    ${EndIf}

  bdo_reparse_tree_next:
    ClearErrors
    FindNext $R1 $R2
    ${If} ${Errors}
      ; NSIS reports normal end-of-enumeration through the error flag.
      StrCpy $R2 ""
    ${EndIf}
    Goto bdo_reparse_tree_loop

  bdo_reparse_tree_done:
  FindClose $R1
  ClearErrors
  Pop $5
  Pop $4
  Pop $R3
  Pop $R2
  Pop $R1
  Pop $R0
FunctionEnd

!macro BDO_HARDEN_INSTALL_DIR
  !insertmacro BDO_EXPECTED_INSTALL_DIR
  ${If} $INSTDIR != $R8
    Abort "BDO Optimizer는 보호된 기본 Program Files 경로에만 설치할 수 있습니다."
  ${EndIf}
  !insertmacro BDO_REJECT_REPARSE_INSTALL_DIR
  ClearErrors
  CreateDirectory "$INSTDIR"
  ${If} ${Errors}
    Abort "BDO Optimizer 설치 디렉터리를 만들 수 없습니다."
  ${EndIf}
  !insertmacro BDO_REJECT_REPARSE_INSTALL_DIR
  Push "$INSTDIR"
  Call BDO_RejectReparseTree
  ExecWait '"$SYSDIR\icacls.exe" "$INSTDIR" /setowner "*S-1-5-32-544" /T /L /Q' $R9
  ${If} $R9 <> 0
    Abort "BDO Optimizer 설치 디렉터리 owner 강화에 실패했습니다."
  ${EndIf}
  ExecWait '"$SYSDIR\icacls.exe" "$INSTDIR" /reset /T /L /Q' $R9
  ${If} $R9 <> 0
    Abort "BDO Optimizer 설치 디렉터리 ACL 초기화에 실패했습니다."
  ${EndIf}
  ExecWait '"$SYSDIR\icacls.exe" "$INSTDIR" /verify /T /L /Q' $R9
  ${If} $R9 <> 0
    Abort "BDO Optimizer 설치 디렉터리 ACL 검증에 실패했습니다."
  ${EndIf}
  Push "$INSTDIR"
  Call BDO_RejectReparseTree
!macroend

!macro NSIS_HOOK_PREINSTALL
  !insertmacro BDO_HARDEN_INSTALL_DIR
!macroend

!macro NSIS_HOOK_POSTINSTALL
  !insertmacro BDO_HARDEN_INSTALL_DIR
!macroend

!macro BDO_TASK_FILE_EXISTS TASK_NAME RESULT_VAR
  ${DisableX64FSRedirection}
  ${If} ${FileExists} "$WINDIR\System32\Tasks\${TASK_NAME}"
    StrCpy ${RESULT_VAR} 1
  ${Else}
    StrCpy ${RESULT_VAR} 0
  ${EndIf}
  ${EnableX64FSRedirection}
!macroend

!macro BDO_DELETE_TASK_FAIL_CLOSED TASK_NAME
  nsExec::ExecToLog '"$SYSDIR\schtasks.exe" /Query /TN "${TASK_NAME}"'
  Pop $R9
  ${If} $R9 == 0
    nsExec::ExecToLog '"$SYSDIR\schtasks.exe" /Delete /TN "${TASK_NAME}" /F'
    Pop $R9
    nsExec::ExecToLog '"$SYSDIR\schtasks.exe" /Query /TN "${TASK_NAME}"'
    Pop $R9
    ${If} $R9 == 0
      Abort "BDO Optimizer 예약 작업 삭제에 실패했습니다: ${TASK_NAME}"
    ${Else}
      !insertmacro BDO_TASK_FILE_EXISTS "${TASK_NAME}" $R8
      ${If} $R8 == 1
        Abort "BDO Optimizer 예약 작업 파일이 남아 있습니다: ${TASK_NAME}"
      ${EndIf}
    ${EndIf}
  ${Else}
    !insertmacro BDO_TASK_FILE_EXISTS "${TASK_NAME}" $R8
    ${If} $R8 == 1
      Abort "BDO Optimizer 예약 작업 조회에 실패했습니다: ${TASK_NAME}"
    ${Else}
      DetailPrint "이미 없는 예약 작업: ${TASK_NAME}"
    ${EndIf}
  ${EndIf}
!macroend

!macro NSIS_HOOK_PREDELETE
  ${If} $UpdateMode = 1
    DetailPrint "BDO Optimizer 업데이트: 예약 작업을 보존합니다."
  ${Else}
    DetailPrint "BDO Optimizer 예약 작업 정리 중..."
    !insertmacro BDO_DELETE_TASK_FAIL_CLOSED "BDO_Optimizer_Launcher_Autostart"
    !insertmacro BDO_DELETE_TASK_FAIL_CLOSED "BDO_Auto_Shutdown_Once"
    !insertmacro BDO_DELETE_TASK_FAIL_CLOSED "BDO_Auto_Shutdown_Weekly"
  ${EndIf}
!macroend
