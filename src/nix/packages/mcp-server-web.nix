{ inputs, ... }:

{
  self.overlays.overlays.mcp-server-web = {
    flake = true;
    value =
      final: _prev:
      let
        inherit (final) lib;

        root = ../../..;

        manifest = builtins.fromTOML (builtins.readFile ../../crates/mcp-server-web/Cargo.toml);

        # NOTE: only linux gets the musl target - there is no static darwin
        isLinux = final.stdenv.hostPlatform.isLinux;
        target = "${final.stdenv.hostPlatform.parsed.cpu.name}-unknown-linux-musl";
        targetUnderscored = builtins.replaceStrings [ "-" ] [ "_" ] target;

        # NOTE: C dependencies (aws-lc) must be compiled and linked against
        # musl and not glibc
        musl = final.pkgsStatic.stdenv.cc;
        muslCc = "${musl}/bin/${musl.targetPrefix}cc";

        # NOTE: haraspkgs overlays are not available to downstream overlays
        # so this is what its rust helpers do, using its inputs
        rustBin = inputs.haraspkgs.inputs.rust-overlay.lib.mkRustBin { } final;

        toolchain = rustBin.stable.latest.default.override {
          extensions = [
            "rustfmt"
            "clippy"
            "rust-analyzer"
          ];
          targets = lib.optional isLinux target;
        };

        craneLib = (inputs.haraspkgs.inputs.crane.mkLib final).overrideToolchain (_: toolchain);

        src = craneLib.cleanCargoSource (
          lib.fileset.toSource {
            inherit root;
            fileset = lib.fileset.unions [
              (root + "/Cargo.toml")
              (root + "/Cargo.lock")
              ../../crates
            ];
          }
        );

        # NOTE: used by the dev shell to write .cargo/config.toml so that
        # cargo works offline
        cargoVendorDir = craneLib.vendorCargoDeps { inherit src; };

        commonArgs = {
          inherit src cargoVendorDir;
          pname = "mcp-server-web";
          inherit (manifest.package) version;
          strictDeps = true;
          cargoExtraArgs = "-p mcp-server-web";
          # NOTE: the tests build a tls client, which needs the system roots
          # and the build sandbox has none
          env = {
            SSL_CERT_FILE = "${final.cacert}/etc/ssl/certs/ca-bundle.crt";
          }
          // lib.optionalAttrs isLinux {
            CARGO_BUILD_TARGET = target;
            CARGO_BUILD_RUSTFLAGS = "-C target-feature=+crt-static";
            "CC_${targetUnderscored}" = muslCc;
            "CARGO_TARGET_${lib.toUpper targetUnderscored}_LINKER" = muslCc;
          };
        };

        cargoArtifacts = craneLib.buildDepsOnly commonArgs;
      in
      {
        mcp-server-web = craneLib.buildPackage (
          commonArgs
          // {
            inherit cargoArtifacts;

            passthru = {
              inherit toolchain craneLib cargoVendorDir;
            };

            meta = {
              description = manifest.package.description;
              license = lib.licenses.mit;
              mainProgram = "mcp-server-web";
            };
          }
        );
      };
  };
}
