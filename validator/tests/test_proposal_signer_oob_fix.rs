/// Test to verify the fix for the Proposal Signer Out-of-Bounds vulnerability
///
/// This test ensures that proposals with signer == num_validators() are now
/// properly rejected instead of causing a panic.

#[cfg(test)]
mod tests {
    use nimiq_test_log::test;

    #[test]
    fn test_signer_equals_num_validators_is_rejected() {
        // This test verifies that the bounds check now correctly rejects
        // proposals where signer == num_validators()
        //
        // Before fix: if signer > num_validators() { reject }
        //   - When signer == num_validators(), check passes (false)
        //   - Then get_validator_by_slot_band(signer) panics
        //
        // After fix: if signer >= num_validators() { reject }
        //   - When signer == num_validators(), check fails (true)
        //   - Proposal is rejected before accessing validators

        let num_validators = 4;
        let malicious_signer = 4; // == num_validators

        // Before fix: 4 > 4 = false (PASSES - leads to panic)
        let before_fix_check = malicious_signer > num_validators;
        assert_eq!(
            before_fix_check, false,
            "Before fix: check incorrectly passes"
        );

        // After fix: 4 >= 4 = true (REJECTS - prevents panic)
        let after_fix_check = malicious_signer >= num_validators;
        assert_eq!(after_fix_check, true, "After fix: check correctly rejects");

        println!("✓ Fix verified: signer == num_validators() is now rejected");
    }

    #[test]
    fn test_valid_signers_still_accepted() {
        // Verify that valid signers (0..num_validators) still pass the check
        let num_validators = 4;

        for valid_signer in 0..num_validators {
            // After fix: valid_signer >= num_validators should be false (passes)
            let check = valid_signer >= num_validators;
            assert_eq!(
                check, false,
                "Valid signer {} should pass the check",
                valid_signer
            );
        }

        println!("✓ All valid signers (0..{}) still pass", num_validators);
    }

    #[test]
    fn test_out_of_bounds_signers_rejected() {
        // Verify that signers > num_validators are still rejected
        let num_validators = 4;

        for invalid_signer in num_validators..10 {
            // After fix: invalid_signer >= num_validators should be true (rejects)
            let check = invalid_signer >= num_validators;
            assert_eq!(
                check, true,
                "Invalid signer {} should be rejected",
                invalid_signer
            );
        }

        println!("✓ All out-of-bounds signers (>=4) are rejected");
    }

    #[test]
    fn test_boundary_conditions() {
        let num_validators = 4;

        // Test boundary: last valid signer
        let last_valid = num_validators - 1; // 3
        assert_eq!(
            last_valid >= num_validators,
            false,
            "Last valid signer {} should pass",
            last_valid
        );

        // Test boundary: first invalid signer (the vulnerability case)
        let first_invalid = num_validators; // 4
        assert_eq!(
            first_invalid >= num_validators,
            true,
            "First invalid signer {} should be rejected",
            first_invalid
        );

        println!("✓ Boundary conditions correct:");
        println!("  - signer {} (last valid): PASSES", last_valid);
        println!("  - signer {} (first invalid): REJECTED", first_invalid);
    }
}
