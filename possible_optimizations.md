Performance optimizations:
1) Changing the try-and-increment function to use binary addition instead of Fp addition. This should give a significant speed-up.
2) Don't add the public keys twice (one for the hash-to-g1 and another for the aggregate-pk). Instead add them into two parts: signers and non-signers. Them you feed the signers part into the aggregate-pk, and feed both parts into the hash-to-g1 (where a single add will give you the full sum). However, this requires that the macro block gadget also receives the signer's bitmap for the next block.
3) Instead of requiring the full list of public keys as a public input, require only the aggregate public key that will be used to verify the last block header (off-circuit). The prover can just provide the needed bitmap as a private input.

Security optimizations:
1) During the public key addition we are first adding the generator and then subtracting it at the end. Either find a different workaround or hard-code a different generator with an unknown secret key (using public randomness and hash-to-curve). 
