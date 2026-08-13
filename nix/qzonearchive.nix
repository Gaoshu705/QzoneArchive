{ pkgs }:
let
  lib = pkgs.lib;
  stdenv = pkgs.stdenv;
  rustPlatform = pkgs.rustPlatform;
  cargoTauri = pkgs.cargo-tauri;
  nodejs = pkgs.nodejs_22;
in
rustPlatform.buildRustPackage {
  pname = "qzonearchive";
  version = "1.0.3";

  src = lib.cleanSource ../.;

  cargoRoot = "src-tauri";
  buildAndTestSubdir = "src-tauri";

  cargoHash = "sha256-9bdsDzJZOIF4nVQGib8vafv8Eb8pQRSfzMMyaXJywzs=";

  npmDeps = pkgs.fetchNpmDeps {
    name = "qzonearchive-1.0.3-npm-deps";
    src = lib.cleanSource ../.;
    hash = "sha256-24RbBcv3OY1LvWCaXDbvT5Bou3uw9imus5iroD1WUF4=";
  };

  nativeBuildInputs = [
    nodejs
    pkgs.npmHooks.npmConfigHook
    pkgs.pkg-config
    pkgs.cmake
    cargoTauri.hook
  ] ++ lib.optionals stdenv.hostPlatform.isLinux [
    pkgs.wrapGAppsHook4
  ];

  buildInputs = lib.optionals stdenv.hostPlatform.isLinux [
    pkgs.glib-networking
    pkgs.openssl
    pkgs.sqlite
    pkgs.xdotool
    pkgs.webkitgtk_4_1
    pkgs.glib
    pkgs.gtk3
    pkgs.libsoup_3
    pkgs.patchelf
    pkgs.gst_all_1.gst-plugins-base
    pkgs.gst_all_1.gst-plugins-good
    pkgs.gst_all_1.gst-plugins-bad
    pkgs.gst_all_1.gst-plugins-ugly
    pkgs.gst_all_1.gst-libav
  ];

  preFixup = lib.optionalString stdenv.hostPlatform.isLinux ''
    gappsWrapperArgs+=(
      --set FONTCONFIG_FILE ${pkgs.makeFontsConf {
        fontDirectories = [ pkgs.noto-fonts-cjk-sans ];
      }}
    )
  '';

  doCheck = false;

  postInstall = ''
    install -Dm644 "$src/src-tauri/icons/icon.png" "$out/share/icons/hicolor/512x512/apps/qzonearchive.png"

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
  '';
}
