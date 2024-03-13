# Deterministic Build System for core-rs-albatross

## Software Integrity and Authenticity
Software integrity and authenticity focuses on ensuring that software has not been modified at any point between the software maintainer and the end user. It is achieved by combining deterministic builds to make integrity checks simple, along with public key cryptography to verify the authenticity of the software.

### Deterministic Builds
Bit-for-bit reproducible builds, also known as deterministic builds, are a security measure which helps protect the integrity of software, making it simple to verify that there have been no changes introduced to the software maliciously or otherwise.

It offers protection from supply chain and other attacks that modify or introduce malicious software into otherwise legitimate software. Often the origin of such attacks is compromised software maintainers, server side attacks, and other forms of attacks such as MITM. 

Determinism is achieved by 
* Using a docker image for building of software
* Pinning core-rs-albatross in Dockerfile using commit hash 
* Hash-locking all software dependency versions (rust, debian packages, docker base image)
* Setting `SOURCE_DATE_EPOCH=1` which instruct the build to set the timestamps to 0
* Setting rust compiler flags `-C target-feature=-crt-static -C codegen-units=1` which limit the number of compute units used during the build process and ensure the build is done dynamically

These steps ensure that the build always outputs the exact same software, down to the last bit.

Multiple individuals may independently reproduce the software to ensure that the software hasn't been compromised on one of the machines.

This build setup also leverages [stagex](https://codeberg.org/stagex/stagex), which ensures that the entire build root used for building `core-rs-albatross` is deterministic and fully verifiable.

### Signed Software
In addition to deterministic builds, guarantees that trusted parties are the originators of the software is important. For this, public key infrastructure, namely PGP, is used. Multiple software maintainers independently build the software deterministically on their own machines, and sign the resulting software using their [well known PGP public keys](todo link to github and keys.nimiq.com), then include those signed digests with the latest release. 

An individual software maintainer creates a single point of failure, where if they are compromised, the software supply chain security is broken. The likelihood of multiple software maintainers being compromised at the same time is much lower, giving the end user multiple points of reference they can check to achieve a reasonable level of confidence that the software they are using has not been tampered, and is authentic.

### Verifying Software Independently
Required software:
* `docker` v =>25.10.22
    * Use `containerd` backend:
        * Edit the daemon.json with:
        ```
        {
            "features": {
                "containerd-snapshotter": true
            }
        } 
        ```
    * docker buildx create --name oci-builder  --driver docker-container --node oci-builder
    * docker buildx use oci-builder


#### 1. Build the software
Currently only AMD architecture is supported.

```
# Get new tags from remote
git fetch --tags

# Get latest tag name
latestTag=$(git describe --tags "$(git rev-list --tags --max-count=1)")

# Checkout latest tag
git checkout $latestTag

# Move to build/ dir
cd build

# Build software
make build
```
Build the rust code deterministically. It will output the release to `/target/release`, along with digests. It may take some time to build. Once it is built, you may verify the software.

#### 2. Import Nimiq team PGP Keys
Before being able to verify the signatures, download the Nimiq pgp keys from [keyring.nimiq.com](https://keyring.nimiq.com). Ensure you download all keys present. Additional checks of the keys by referring to the team's GitHub page which also includes the keys in the releases is recommended to ensure you are using a Nimiq owned key. Then import the keys into your keyring using:
```
# Import the keys
gpg --import pub-0.asc pub-1.asc pub-2.asc
```

#### 3. Run verification script
You can now verify the software you generated matches the signed hashes of builds the Nimiq team produced:
```
cd build/
make verify-software
```

If you see a warning which resembles the following ensure you immediately report this to security@nimiq.com:
```
WARNING ...
=============================================
...
WARNING ==================
```

Additional information:
* [QubesOS Software Verification Guide](https://www.qubes-os.org/security/verifying-signatures/)

---

## Developer Docs

### Generate and sign build digest
In order to sign the build, run:
```
cd build/
make sign-digests <pgp_key_id>
```

It will generate a PGP signed digest in `./release/digests/<pgp_key_id>` which can be made part of releases.

### Generate PGP keys and seed Yubikey:
Guide can be found on https://book.hashbang.sh/docs/security/key-management/gnupg/

There are two flows:
* Generate keypair on yubikey (no backup)
* Generate keypair in TEE (airgap, or hardware based VM) (backed up)

### Debugging
* [`diffoscope`](https://github.com/anthraxx/diffoscope) can be used to check if builds are not deterministic to find the difference in the files

#### Helpful Docker commands
```
docker create --name <container_name> <image_name>
```

Copy from container volume to local dir:
```
docker cp container_name:WORKDIR ./<local_path>
```

Inspect image by creating a shell in it:
```
docker run -it --rm deterministic-rust-build /bin/sh
```

Get docker image digest:
```
docker pull debian:bullseye
```

#### GPG / Smart Card
* If your smart card isn't "responsive":
    * `gpg --card-status` can help reconnect it
