Performance optimizations:
1) Changing the try-and-increment function to use binary addition instead of Fp addition. This should give a significant speed-up.
2) Instead of requiring the full list of public keys as a public input, require only the aggregate public key that will be used to verify the last block header (off-circuit). The prover can just provide the needed bitmap as a private input.

Security optimizations:
1) During the public key addition we are first adding the generator and then subtracting it at the end. Either find a different workaround or hard-code a different generator with an unknown secret key (using public randomness and hash-to-curve). 
