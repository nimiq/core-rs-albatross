BUILD_IMAGE_NAME := det-core-rs-albatross-image
TEMP_CONTAINER_NAME := det-core-rs-albatross-container

WORKDIR := /home/nimiq

TARGET_ARCHITECTURE := linux/amd64

.PHONY: all
all: build

.PHONY: build-image
build-image:
	cd .. && \
	docker build -t $(BUILD_IMAGE_NAME) -f build/Containerfile .

.PHONY: build
build: build-image copy-artifacts create-digests

.PHONY: temp-container
temp-container:
	echo "Creating container $(TEMP_CONTAINER_NAME) from image $(BUILD_IMAGE_NAME)"
	docker create --name $(TEMP_CONTAINER_NAME) $(BUILD_IMAGE_NAME) tail -f /dev/null

.PHONY: copy-artifacts
copy-artifacts: clean-temp-container temp-container 
	mkdir -p ../target/release
	docker cp $(TEMP_CONTAINER_NAME):$(WORKDIR)/target/release ../target

.PHONY: clean-image
clean-image:
	docker rmi -f $(BUILD_IMAGE_NAME) || true

.PHONY: clean-temp-container
clean-temp-container:
	docker rm -f $(TEMP_CONTAINER_NAME) || true

.PHONY: clean-all
clean-all: clean-image clean-temp-container

.PHONY: verify-software
verify-software:
	./scripts/check-digests.sh
	./scripts/check-signatures.sh

.PHONY: create-digests
create-digests:
	./scripts/create-digests.sh

.PHONY: sign-digests
sign-digests:
	./scripts/sign-digests.sh $(arg)