.PHONY: build release test fmt clippy clean install dist

# 本地开发构建
build:
	cargo build

test:
	cargo test

fmt:
	cargo fmt

clippy:
	cargo clippy --all-targets -- -D warnings

# 本地 release 二进制
release:
	cargo build --release

# 安装到 ~/.cargo/bin
install:
	cargo install --path crates/cli

clean:
	cargo clean
	rm -rf dist

# 跨平台交叉编译（需要预装对应 target）
# 产物命名与 install.sh 对齐：reviewgate-<os>-<arch>
TARGETS = \
	x86_64-unknown-linux-gnu:linux-x64 \
	aarch64-unknown-linux-gnu:linux-arm64 \
	x86_64-apple-darwin:darwin-x64 \
	aarch64-apple-darwin:darwin-arm64

dist:
	mkdir -p dist
	@for pair in $(TARGETS); do \
		target=$${pair%%:*}; name=$${pair##*:}; \
		echo "=> $$target -> reviewgate-$$name"; \
		rustup target add $$target >/dev/null 2>&1 || true; \
		cargo build --release --target $$target -p reviewgate-cli && \
		cp target/$$target/release/reviewgate dist/reviewgate-$$name; \
	done
	cd dist && shasum -a 256 reviewgate-* > sha256sum.txt
	@echo "产物在 dist/"
