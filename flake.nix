{
  description = "TODO Description";
  inputs = {
    nixpkgs.url = github:nixos/nixpkgs;
    flake-utils = {
      url = github:numtide/flake-utils;
      inputs.nixpkgs.follows = "nixpkgs";
    };
    naersk = {
      url = github:nix-community/naersk;
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    { self
    , nixpkgs
    , flake-utils
    , naersk
    }:
    flake-utils.lib.eachDefaultSystem (system:
    let
      lib = nixpkgs.lib.${system};
      pkgs = nixpkgs.legacyPackages.${system};
      rustTools = import ./nix/rust.nix {
        nixpkgs = pkgs;
      };
      getRust =
        { channel ? "nightly"
        , date
        , sha256
        , targets ? [ "wasm32-unknown-unknown" "wasm32-wasi" "wasm32-unknown-emscripten" ]
        }: (rustTools.rustChannelOf {
          inherit channel date sha256;
        }).rust.override {
          inherit targets;
          extensions = [ "rust-src" "rust-analysis" ];
        };
      # This is the version used across projects
      rust = getRust { date = "2022-02-20"; sha256 = "sha256-ZptNrC/0Eyr0c3IiXVWTJbuprFHq6E1KfBgqjGQBIRs="; };
      # Get a naersk with the input rust version
      naerskWithRust = rust: naersk.lib."${system}".override {
        rustc = rust;
        cargo = rust;
      };
      # Naersk using the default rust version
      naerskDefault = naerskWithRust rust;
      buildRustProject = pkgs.makeOverridable ({ rust, naersk ? naerskWithRust rust, ... } @ args: naersk.buildPackage ({
        buildInputs = with pkgs; [ ];
        targets = [ ];
        copyLibs = true;
        remapPathPrefix =
          true; # remove nix store references for a smaller output package
      } // args));

      # Convenient for running tests
      testProject = buildRustProject { doCheck = true; inherit rust root; };
      # Load a nightly rust. The hash takes precedence over the date so remember to set it to
      # something like `lib.fakeSha256` when changing the date.
      crateName = "my-crate";
      root = ./.;
      # This is a wrapper around naersk build
      # Remember to add Cargo.lock to git for naersk to work
      project = buildRustProject {
        inherit root rust;
      };
      lurk-example = project.override {
        cargoBuildOptions = d: d ++ [ "--example lurk" ];
        copySources = [ "examples" "src" ];
        copyBins = true;
      };
    in
    {
      packages = {
        ${crateName} = project;
        "${crateName}-test" = testProject;
      };

      defaultPackage = self.packages.${system}.${crateName};

      # `nix develop`
      devShell = pkgs.mkShell {
        inputsFrom = builtins.attrValues self.packages.${system};
        nativeBuildInputs = [ rust ];
        buildInputs = with pkgs; [
          rust-analyzer
          clippy
          rustfmt
          emscripten
        ];
      };
    });
}
