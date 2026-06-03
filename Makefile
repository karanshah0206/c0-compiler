default: release

release:
	cd compiler && cargo build --release
	mkdir -p bin
	cp compiler/target/release/compiler bin/c0mpile

clean:
	cd compiler && cargo clean
	rm -rf bin

.PHONY: release clean
