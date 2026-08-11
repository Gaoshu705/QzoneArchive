{ pkgs }:
let
  lib = pkgs.lib;
  stdenv = pkgs.stdenv;
in
stdenv.mkDerivation {
  pname = "qzonearchive";
  version = "1.0.3";

  src = ../dist/nix-input;

  installPhase = ''
    runHook preInstall

    install -Dm755 bin/qzonearchive "$out/bin/qzonearchive"

    install -Dm644 icons/icon.png "$out/share/icons/hicolor/512x512/apps/qzonearchive.png"

    mkdir -p "$out/share/applications"
    cat > "$out/share/applications/qzonearchive.desktop" <<EOF
[Desktop Entry]
Type=Application
Name=空间归档
Comment=本地 QQ 空间归档工具
Exec=$out/bin/qzonearchive
Icon=qzonearchive
Categories=Utility;
EOF

    runHook postInstall
  '';
}
