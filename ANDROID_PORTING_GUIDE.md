# FigCraft Android ARM64 태블릿 포팅 가이드 (V6_hotfix)

이 패키지는 [storytold/figcraft](https://github.com/storytold/figcraft)를 본인 GitHub 저장소로 포크(Fork)해 올려두고, **Codemagic 클라우드 빌드(`codemagic.yaml`)**로 안드로이드 ARM64 태블릿용 네이티브 APK를 생성하도록 구성한 **V6_hotfix 최종 안정화 버전**입니다.

## V6_hotfix 패치 내역

- **캔버스 전용 제스처 격리 (`touch.rs`, `shell.rs`)**: `canvas_rect`를 도입해 `Hand` 팬, 2손가락 줌·팬, 550ms 롱프레스, 24pt 더블탭 스냅이 오직 캔버스 영역 내부에서만 작동하도록 한정했습니다. `Hand` 모드가 켜져 있어도 상단 파일 탭·툴바·좌측 레이어·우측 디자인 패널 클릭이 정상 동작하며, 인접 아이콘 오스냅이 발생하지 않습니다.
- **원터치 UI 배율 조절 버튼 (`UI: 100% / 125% / 140%`)**: 상단 바 버튼으로 `ctx.set_zoom_factor`를 즉시 전환하며, 마지막 배율은 `android_shell_prefs.json`에 자동 저장됩니다(13.3인치 태블릿 기본값 `125%`).
- **몰입형 전체화면 온/오프 (`Full` 버튼)**: 상단 바 `Full` 버튼으로 Android 시스템 상태바·내비게이션 바 숨김(Immersive Mode)을 자유롭게 켜고 끌 수 있으며 설정이 자동 유지됩니다.
- **`Paste` 포커스 복원 & 클립보드 MIME 폴백**: `Paste` 버튼 터치 시 직전 텍스트 필드 포커스를 복원한 뒤 `egui::Event::Paste`를 주입하며, Java에서 `ClipDescription` MIME 폴백 및 2 MB 텍스트 안전 상한을 적용했습니다.
- **상단 바 터치 타겟 확대**: 상단 바 버튼 최소 높이를 `34 pt`로 키워 태블릿 손가락 터치 정확도를 높였습니다.

## GitHub 업로드 및 Codemagic 빌드 절차

1. GitHub에서 [storytold/figcraft](https://github.com/storytold/figcraft)를 본인 계정으로 **Fork** 합니다.
2. 이 ZIP 압축을 풀어 포크한 저장소 루트에 그대로 복사합니다.
3. 저장소 루트 `Cargo.toml`의 `[workspace]` `members`에 `"apps/figcraft-android"`를 추가합니다.
4. `crates/ui-egui/Cargo.toml`에서 Android 타겟일 때 데스크톱 전용 `rfd`, `accesskit` 의존성이 제외되도록 확인합니다.
5. `android-port` 브랜치에 Push한 뒤 **Codemagic**에서 `figcraft-android-arm64` 워크플로우로 빌드하고 Artifacts의 `app-debug.apk`를 태블릿에 설치합니다.
