{
  description = "Native Linux viewer and editor for imported OneNote notebooks";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs =
    {
      self,
      nixpkgs,
      ...
    }:
    let
      supportedSystems = [
        "x86_64-linux"
        "aarch64-linux"
      ];
      forAllSystems = nixpkgs.lib.genAttrs supportedSystems;
    in
    {
      packages = forAllSystems (
        system:
        let
          pkgs = import nixpkgs { inherit system; };
        in
        {
          default = pkgs.rustPlatform.buildRustPackage {
            pname = "onenote-linux";
            version = "0.2.0";
            src = pkgs.lib.fileset.toSource {
              root = ./.;
              fileset = pkgs.lib.fileset.unions [
                ./Cargo.toml
                ./Cargo.lock
                ./src
                ./packaging
              ];
            };

            cargoHash = "sha256-7TeCDUjfC0/VTcrRinULBhIg6+WH6jyDUmXepHOcoF8=";

            nativeBuildInputs = [
              pkgs.makeWrapper
              pkgs.pkg-config
            ];
            buildInputs = [
              pkgs.libGL
              pkgs.libxkbcommon
              pkgs.wayland
              pkgs.libx11
              pkgs.libxcursor
              pkgs.libxi
              pkgs.libxrandr
            ];

            postInstall = ''
              install -Dm644 packaging/io.github.onenote-linux.Viewer.desktop \
                "$out/share/applications/io.github.onenote-linux.Viewer.desktop"
              install -Dm644 packaging/io.github.onenote-linux.Viewer.svg \
                "$out/share/icons/hicolor/scalable/apps/io.github.onenote-linux.Viewer.svg"
              install -Dm644 packaging/io.github.onenote-linux.Viewer.xml \
                "$out/share/mime/packages/io.github.onenote-linux.Viewer.xml"
            '';

            postFixup = ''
              wrapProgram "$out/bin/onenote-linux" \
                --prefix LD_LIBRARY_PATH : ${
                  pkgs.lib.makeLibraryPath [
                    pkgs.libGL
                    pkgs.libxkbcommon
                    pkgs.wayland
                    pkgs.libx11
                    pkgs.libxcursor
                    pkgs.libxi
                    pkgs.libxrandr
                  ]
                }
            '';

            meta = {
              description = "Native Linux viewer and editor for imported OneNote notebooks";
              homepage = "https://github.com/PatrykFlama/OneNote-Linux";
              license = pkgs.lib.licenses.mit;
              mainProgram = "onenote-linux";
              platforms = pkgs.lib.platforms.linux;
            };
          };
        }
      );

      apps = forAllSystems (
        system:
        let
          package = self.packages.${system}.default;
        in
        {
          default = {
            type = "app";
            program = "${package}/bin/onenote-linux";
            meta.description = "View and edit imported OneNote notebooks";
          };
        }
      );

      devShells = forAllSystems (
        system:
        let
          pkgs = import nixpkgs { inherit system; };
        in
        {
          default = pkgs.mkShell {
            packages = [
              pkgs.cargo
              pkgs.clippy
              pkgs.rustc
              pkgs.rustfmt
              pkgs.pkg-config
            ];
            buildInputs = [
              pkgs.libGL
              pkgs.libxkbcommon
              pkgs.wayland
              pkgs.libx11
              pkgs.libxcursor
              pkgs.libxi
              pkgs.libxrandr
            ];
            LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath [
              pkgs.libGL
              pkgs.libxkbcommon
              pkgs.wayland
              pkgs.libx11
              pkgs.libxcursor
              pkgs.libxi
              pkgs.libxrandr
            ];
          };
        }
      );
    };
}
