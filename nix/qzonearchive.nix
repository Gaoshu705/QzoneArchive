{ pkgs }:
let
  nodejs = pkgs.nodejs_20;
  lib = pkgs.lib;
  stdenv = pkgs.stdenv;
in
stdenv.mkDerivation {
  pname = "qzonearchive";
  version = "1.0.3";

  src = lib.cleanSource ../.;

  nativeBuildInputs = with pkgs; [
    nodejs
    pkg-config
    cmake
    gcc
    gnumake
    rustc
    cargo
    perl
    wrapGAppsHook3
  ];

  buildInputs = with pkgs; [
    glib
    gtk3
    webkitgtk_4_1
    libsoup_3
    openssl
    patchelf
    sqlite
    xdotool
    gst_all_1.gst-plugins-base
    gst_all_1.gst-plugins-good
    gst_all_1.gst-plugins-bad
  ];

  configurePhase = ''
    npm ci
  '';

  buildPhase = ''
    npm run build
    npx tauri build --no-bundle --ci
  '';

  installPhase = ''
    runHook preInstall

    install -Dm755 src-tauri/target/release/qzonearchive "$out/bin/qzonearchive"

    install -Dm644 src-tauri/icons/icon.png "$out/share/icons/hicolor/512x512/apps/qzonearchive.png"

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
