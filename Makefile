.PHONY: core test verify-core desktop desktop-preview desktop-test packaging-test theme theme-check mobile-core mobile-prebuild mobile-test mobile-ios mobile-native-test

core:
	scripts/with-toolchain.sh cargo build --manifest-path core/Cargo.toml --release --lib

test:
	scripts/with-toolchain.sh cargo test --manifest-path core/Cargo.toml
	scripts/with-toolchain.sh cargo test --manifest-path core/keyboard/Cargo.toml

verify-core: core
	$(MAKE) test

desktop: theme
	cd apps/desktop && npm ci && ../../scripts/with-toolchain.sh npx tauri build

desktop-preview: theme
	scripts/build-desktop-preview.sh

desktop-test: theme
	cd apps/desktop && npm ci && npm run build
	scripts/with-toolchain.sh cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml

packaging-test:
	python3 -B -m unittest discover -s scripts/tests -p 'test_desktop_*.py'

theme:
	python3 scripts/generate-theme.py

theme-check:
	python3 scripts/generate-theme.py --check

IOS_DESTINATION ?= platform=iOS Simulator,name=iPhone 17
PENNEL_PODS_VERSION := $(shell sed -nE 's/^COCOAPODS: ([0-9]+\.[0-9]+\.[0-9]+)$$/\1/p' apps/mobile/Podfile.lock)

mobile-core:
	cd apps/mobile && npm run native:prepare

mobile-prebuild: mobile-core
	cd apps/mobile && npm run test:prebuild
	cd apps/mobile && npm run prebuild:clean
	cp apps/mobile/Podfile.lock apps/mobile/ios/Podfile.lock
	cd apps/mobile/ios && pod '_$(PENNEL_PODS_VERSION)_' install --deployment
	cd apps/mobile && npm run test:pods

mobile-test:
	cd apps/mobile && npm run typecheck && npm run test:plugins && npm run test:widgets && npm run test:ui
	swift test --package-path apps/mobile/native/ios/MobileKit

mobile-ios: mobile-prebuild
	cd apps/mobile && npm run build:ios

mobile-native-test:
	bash scripts/ci-xcodebuild.sh test -workspace apps/mobile/ios/Pennel.xcworkspace -scheme Pennel -configuration Debug \
		-destination '$(IOS_DESTINATION)' -derivedDataPath apps/mobile/build/TestDerivedData \
		CODE_SIGNING_ALLOWED=YES CODE_SIGN_IDENTITY=- CODE_SIGN_STYLE=Manual DEVELOPMENT_TEAM=
