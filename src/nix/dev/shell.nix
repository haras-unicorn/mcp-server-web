{ selfLib, ... }:

{
  self.devShells.default = {
    flake = true;
    value =
      {
        config,
        lib,
        pkgs,
        ...
      }:
      let
        yaml = pkgs.formats.yaml { };

        toml = pkgs.formats.toml { };

        json = pkgs.formats.json { };

        harasPackages = pkgs.harasPackages;

        # NOTE: the toolchain and vendored crates come from the package so the
        # dev shell and the nix build always agree
        mcpWeb = pkgs.${selfLib.overlays.namespace}.mcp-server-web;
        inherit (mcpWeb.passthru) toolchain cargoVendorDir;

        prettierConfig = yaml.generate "prettier.yaml" {
          printWidth = 80;
          tabWidth = 2;
          useTabs = false;
          trailingComma = "all";
          endOfLine = "lf";
          proseWrap = "always";
        };

        cspellConfig = yaml.generate "cspell.yaml" {
          useGitignore = true;
          version = "0.2";
          language = "en";
          ignorePaths = [
            "CHANGELOG.md"
            "Cargo.lock"
            "flake.lock"
          ];
          words = [
            "direnv"
            "envrc"
            "allowlist"
            "bubblewrap"
            "bwrap"
            "cachix"
            "clippy"
            "cooldown"
            "denylist"
            "dtolnay"
            "fileset"
            "haras"
            "haraspkgs"
            "héllo"
            "mdbook"
            "nixfmt"
            "nixos"
            "nixpkgs"
            "notexample"
            "passthru"
            "pkgs"
            "pname"
            "readabilityrs"
            "reqwest"
            "rmcp"
            "RUSTFLAGS"
            "rustfmt"
            "rustls"
            "schemars"
            "searx"
            "searxng"
            "SearXNG"
            "serde"
            "stdenv"
            "taplo"
            "tokio"
            "lanczos"
            "imageops"
            "mktemp"
            "moka"
            "webp"
          ];
        };

        markdownlintConfig = json.generate "markdownlint.json" {
          "$schema" =
            "https://raw.githubusercontent.com/DavidAnson/markdownlint/main/schema/markdownlint-config-schema.json";
          # NOTE: prettier pads tables and keeps long code lines and links
          MD013 = {
            tables = false;
            code_blocks = false;
          };
        };

        helixServers = lib.mapAttrs (
          _: server:
          {
            inherit (server) command;
          }
          // lib.optionalAttrs (server.arguments != [ ]) {
            args = server.arguments;
          }
          // lib.optionalAttrs (server.config != { }) {
            inherit (server) config;
          }
        ) config.languages.servers;

        helixLanguages = lib.mapAttrsToList (
          name: language:
          {
            inherit name;
            auto-format = language.auto-format;
          }
          // lib.optionalAttrs (language.servers != [ ]) {
            language-servers = language.servers;
          }
          // lib.optionalAttrs (language.formatter != null) {
            formatter = {
              inherit (language.formatter) command;
            }
            // lib.optionalAttrs (language.formatter.arguments != [ ]) {
              args = language.formatter.arguments;
            };
          }
        ) config.languages.languages;

        helixLanguagesToml = toml.generate "languages.toml" (
          lib.optionalAttrs (helixServers != { }) {
            language-server = helixServers;
          }
          // lib.optionalAttrs (helixLanguages != [ ]) {
            language = helixLanguages;
          }
        );

        nixfmtWrapped = harasPackages.makeBubblewrapWrapper {
          name = "nixfmt";
          package = pkgs.nixfmt;
        };

        prettierWrapped = harasPackages.makeBubblewrapWrapper {
          name = "prettier";
          package = pkgs.prettier;
          files = {
            ".prettierignore" = {
              text = ''
                flake.lock
                CHANGELOG.md
              '';
            };
          };
        };

        taploWrapped = harasPackages.makeBubblewrapWrapper {
          name = "taplo";
          package = pkgs.taplo;
        };

        cspellWrapped = harasPackages.makeBubblewrapWrapper {
          name = "cspell";
          package = pkgs.cspell;
        };

        hxWrapped = harasPackages.makeBubblewrapWrapper {
          name = "hx";
          package = pkgs.helix;
          files = {
            "~/.cache/helix" = {
              mode = "rw";
            };
            ".helix/languages.toml" = {
              mode = "ro";
              source = helixLanguagesToml;
            };
          };
        };

        external = with pkgs; [
          nil
          nix
          fd
          yaml-language-server
          markdownlint-cli
          marksman
          mdbook
        ];
      in
      {
        packages = [
          harasPackages.flake-root
          toolchain
          nixfmtWrapped
          prettierWrapped
          taploWrapped
          cspellWrapped
          hxWrapped
        ]
        ++ external;

        # NOTE: the tests build a tls client, which needs the system roots
        # and the nix sandbox has none - nix develop drops SSL_CERT_FILE
        # from dev shells, but not SSL_CERT_DIR
        env.SSL_CERT_DIR = "${pkgs.cacert}/etc/ssl/certs";

        bwrap.files."cspell.yaml".source = cspellConfig;

        # NOTE: points cargo at the crates vendored by nix so it works without
        # network access - ignored by git, and not part of the nix build source
        file.".cargo/config.toml".source = "${cargoVendorDir}/config.toml";

        git = {
          enable = true;
          ignore = {
            templates = [
              "Nix"
              "Rust"
            ];
            patterns = ''
              # Repository

              .env
              docs/book
            '';
          };
          attributes = ''
            * text=auto eol=lf
          '';
        };

        editorconfig = {
          enable = true;
          settings."*" = {
            end_of_line = "lf";
            charset = "utf-8";
            max_line_length = 80;
            indent_style = "space";
            indent_size = 2;
            tab_width = 2;
            insert_final_newline = true;
            trim_trailing_whitespace = true;
          };
          settings."*.rs" = {
            indent_size = 4;
            tab_width = 4;
            max_line_length = 100;
          };
        };

        languages = {
          servers = {
            nil.command = "nil";

            rust-analyzer.command = "rust-analyzer";

            taplo = {
              command = "taplo";
              arguments = [
                "lsp"
                "stdio"
              ];
            };

            yaml-language-server = {
              command = "yaml-language-server";
              arguments = [ "--stdio" ];
            };

            marksman = {
              command = "marksman";
              arguments = [ "server" ];
            };
          };

          languages = {
            nix = {
              servers = [ "nil" ];
              formatter = {
                command = lib.getExe pkgs.nixfmt;
                arguments = [
                  "--filename"
                  "%{buffer_name}"
                ];
              };
            };

            rust.servers = [ "rust-analyzer" ];

            toml.servers = [ "taplo" ];

            yaml = {
              servers = [ "yaml-language-server" ];
              formatter = {
                command = lib.getExe pkgs.prettier;
                arguments = [
                  "--config"
                  (builtins.toString prettierConfig)
                  "--parser"
                  "yaml"
                ];
              };
            };

            markdown = {
              servers = [ "marksman" ];
              formatter = {
                command = lib.getExe pkgs.prettier;
                arguments = [
                  "--config"
                  (builtins.toString prettierConfig)
                  "--parser"
                  "markdown"
                ];
              };
            };
          };
        };

        license = {
          enable = true;
          spdx = "MIT";
          path = "LICENSE.md";
          attrs = {
            "<year>" = "2026";
            "<copyright holders>" = "github@haras-unicorn.net";
          };
        };

        codeOfConduct.contributorCovenant = {
          enable = true;
          path = "CODE_OF_CONDUCT.md";
        };

        cli.commands = {
          docs = {
            description = "Build the documentation.";
            packages = [ pkgs.nix ];
            commands = ''nix build ".#docs" --no-link --print-out-paths'';
          };
        };

        workflows = {
          check = {
            on = {
              pull_request = { };
              push.branches = [ "main" ];
            };

            matrix.os = [ "ubuntu-latest" ];

            installable = null;

            steps = [
              { run = "nix flake check --show-trace"; }
              { run = "nix build --show-trace .#mcp-server-web"; }
              { run = "nix build --show-trace .#docs"; }
            ];
          };

          # NOTE: needs a RELEASE_PLZ_TOKEN personal access token (and a
          # CARGO_REGISTRY_TOKEN secret). Tags and releases created with the
          # default GITHUB_TOKEN do not trigger the build workflow.
          release = {
            on.push.branches = [ "main" ];

            permissions = {
              contents = "write";
              "pull-requests" = "write";
            };

            installable = null;
            installNix = false;
            cachix = null;

            steps = [
              { uses = "dtolnay/rust-toolchain@stable"; }
              {
                uses = "release-plz/action@v0.5";
                "with".command = "release";
                env = {
                  GITHUB_TOKEN = "\${{ secrets.RELEASE_PLZ_TOKEN }}";
                  CARGO_REGISTRY_TOKEN = "\${{ secrets.CARGO_REGISTRY_TOKEN }}";
                };
              }
              {
                uses = "release-plz/action@v0.5";
                "with".command = "release-pr";
                env = {
                  GITHUB_TOKEN = "\${{ secrets.RELEASE_PLZ_TOKEN }}";
                };
              }
            ];
          };

          build = {
            on.release.types = [ "published" ];

            permissions.contents = "write";

            matrix.os = [
              "ubuntu-latest"
              "ubuntu-24.04-arm"
            ];

            installable = null;

            steps = [
              {
                run = ''
                  path="$(nix build --no-link --print-out-paths --show-trace .#mcp-server-web)"
                  system="$(nix eval --impure --raw --expr builtins.currentSystem)"
                  cp "$path/bin/mcp-server-web" "mcp-server-web-$system"
                  gh release upload "$TAG" "mcp-server-web-$system" --clobber
                '';
                env = {
                  GH_TOKEN = "\${{ secrets.GITHUB_TOKEN }}";
                  TAG = "\${{ github.event.release.tag_name }}";
                };
              }
            ];
          };
        };

        check.checks = {
          clippy = {
            packages = [ toolchain ];
            commands = ''
              CARGO_HOME="$(mktemp -d)"
              export CARGO_HOME
              trap 'rm -rf "$CARGO_HOME"' EXIT
              cp ${cargoVendorDir}/config.toml "$CARGO_HOME/config.toml"
              cargo clippy --workspace --all-targets -- -D warnings
            '';
          };

          test = {
            packages = [ toolchain ];
            commands = ''
              CARGO_HOME="$(mktemp -d)"
              export CARGO_HOME
              trap 'rm -rf "$CARGO_HOME"' EXIT
              cp ${cargoVendorDir}/config.toml "$CARGO_HOME/config.toml"
              export SSL_CERT_DIR=${pkgs.cacert}/etc/ssl/certs
              cargo test --workspace
            '';
          };

          cspell = {
            packages = [ cspellWrapped ];
            commands = "cspell lint . --no-progress";
          };

          markdownlint = {
            packages = [ pkgs.markdownlint-cli ];
            commands = ''
              markdownlint --config ${markdownlintConfig} --ignore-path .gitignore --ignore CHANGELOG.md --ignore LICENSE.md --ignore docs/src/SUMMARY.md .
            '';
          };

          nil = {
            packages = [
              pkgs.fd
              pkgs.nil
            ];
            commands = "fd --glob '*.nix' . -0 | xargs -0 -r nil diagnostics";
          };

          # NOTE: the linux binary must not need anything outside of itself
          static = lib.mkIf pkgs.stdenv.hostPlatform.isLinux {
            packages = [ pkgs.file ];
            commands = ''
              bin="${lib.getExe mcpWeb}"
              echo "checking $bin for static linkage"
              if ! file -b "$bin" | grep -qE "statically linked|static-pie linked"; then
                echo "$bin is not statically linked:" >&2
                file -b "$bin" >&2
                exit 1
              fi
              # NOTE: ldd exits 0 for static-pie, so look at what it prints
              needed="$(${lib.getBin pkgs.glibc}/bin/ldd "$bin" 2>&1 || true)"
              echo "ldd: $needed"
              if echo "$needed" | grep -qE "=>|ld-linux|ld-musl|\.so"; then
                echo "$bin is dynamically linked" >&2
                exit 1
              fi
            '';
          };
        };

        format.formatters = {
          nixfmt = {
            packages = [ nixfmtWrapped ];
            command = "nixfmt";
            includes = [ "*.nix" ];
            excludes = [ "target/**" ];
          };

          taplo = {
            packages = [ taploWrapped ];
            command = "taplo";
            options = [ "format" ];
            includes = [ "*.toml" ];
            excludes = [ "target/**" ];
          };

          prettier = {
            packages = [ prettierWrapped ];
            command = "prettier";
            options = [
              "--write"
              "--config"
              (builtins.toString prettierConfig)
            ];
            includes = [
              "*.json"
              "*.md"
              "*.yaml"
              "*.yml"
            ];
            excludes = [
              "target/**"
              "flake.lock"
              "CHANGELOG.md"
              "LICENSE.md"
            ];
          };

          rustfmt = {
            packages = [ toolchain ];
            command = "rustfmt";
            options = [
              "--edition"
              "2024"
            ];
            includes = [ "*.rs" ];
            excludes = [ "target/**" ];
          };
        };
      };
  };
}
