{
  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/25.11";
    flake-parts.url = "github:hercules-ci/flake-parts";
  };

  outputs = inputs: inputs.flake-parts.lib.mkFlake { inherit inputs; } {
    systems = [ "x86_64-linux" ];
    perSystem = { pkgs, self', ... }: {
      packages.default = pkgs.buildGoModule {
        name = "datacenter-api";
        src = ./.;
        vendorHash = "sha256-3CjXa2QNQPjCm3KJ1f8Z8kh/VH0Orf3KOAPH5RXffqg=";
        nativeBuildInputs = [ pkgs.makeBinaryWrapper ];
        postConfigure = ''
          cp -rT ${self'.packages.web} web/dist
        '';

        GOFLAGS = [ "-tags=embed" ];

        postInstall = ''
          wrapProgram $out/bin/datacenter-api \
              --set-default GIN_MODE release
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
        nativeBuildInputs = [ pkgs.prefetch-npm-deps ];
      };
    };
  };
}
