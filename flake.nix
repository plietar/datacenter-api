{
  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/25.11";
    crane.url = "github:ipetkov/crane";
    flake-parts.url = "github:hercules-ci/flake-parts";
  };

  outputs = inputs: inputs.flake-parts.lib.mkFlake { inherit inputs; } {
    systems = [ "x86_64-linux" ];
    perSystem = { pkgs, self', ... }: {
      packages.default =
        let craneLib = inputs.crane.mkLib pkgs;
        in craneLib.buildPackage {
          name = "datacenter-api";
          src = craneLib.cleanCargoSource ./.;
          checkInputs = [ pkgs.nix ];
          preConfigure = ''
            mkdir -p web
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
