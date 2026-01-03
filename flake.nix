{
  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/25.11";
    flake-parts.url = "github:hercules-ci/flake-parts";
  };

  outputs = inputs: inputs.flake-parts.lib.mkFlake { inherit inputs; } {
    systems = [ "x86_64-linux" ];
    perSystem = { pkgs, self', ... }: {
      packages.default = pkgs.rustPlatform.buildRustPackage {
        name = "datacenter-api";
        src = ./.;
        cargoLock.lockFile = ./Cargo.lock;

        postConfigure = ''
          cp -rT ${self'.packages.web} web/dist
        '';
      };

      packages.web = pkgs.buildNpmPackage {
        name = "datacenter-web";
        src = ./web;
        npmDepsHash = "sha256-cfqbtLxiu/bUp2RO7aIMWH3DYAswF8xCFzenHYnyOpM=";

        installPhase = ''
          runHook preInstall
          cp -rT dist $out
          runHook postInstall
        '';
      };

      devShells.default = pkgs.mkShell {
        inputsFrom = [ self'.packages.default self'.packages.web ];
        nativeBuildInputs = [
          pkgs.prefetch-npm-deps
          pkgs.cargo
          pkgs.rustfmt
        ];
      };
    };
  };
}
