    // Warm/cold equivalence tests. Included into `mod warm_tests` in
    // `run/warm.rs`.

    #[test]
    fn equivalent_runs_produce_no_divergence() {
        let scenario = json!({"name": "x"});
        let cold = cmd(true, &[], &["before", "after"]);
        let warm = cmd(true, &[], &["after", "before"]);
        assert!(compare_equivalence(&scenario, &cold, &warm).is_none());
    }

    #[test]
    fn extra_warm_failure_is_a_divergence() {
        let scenario = json!({"name": "x"});
        let cold = cmd(true, &[], &["before"]);
        let warm = cmd(false, &["focused_mismatch"], &["before"]);
        let divergence = compare_equivalence(&scenario, &cold, &warm).expect("divergence");
        assert_eq!(divergence["warm_ok"], json!(false));
        assert_eq!(divergence["failures_only_in_warm"], json!(["focused_mismatch"]));
    }

    #[test]
    fn snapshot_name_difference_is_a_divergence() {
        let scenario = json!({"name": "x"});
        let cold = cmd(true, &[], &["before", "after"]);
        let warm = cmd(true, &[], &["before"]);
        let divergence = compare_equivalence(&scenario, &cold, &warm).expect("divergence");
        assert_eq!(divergence["snapshots_only_in_cold"], json!(["after"]));
    }

    #[test]
    fn multiset_difference_respects_counts() {
        let a = vec!["e".to_string(), "e".to_string(), "f".to_string()];
        let b = vec!["e".to_string()];
        assert_eq!(difference(&a, &b), vec!["e".to_string(), "f".to_string()]);
    }

    /// Warm log parity means an engine error seen only warm is a REAL divergence,
    /// not something the equivalence check special-cases away: cold and warm now
    /// derive `godot_log_error` from the same finalizer over the same log shape.
    #[test]
    fn warm_only_log_error_is_a_divergence() {
        let scenario = json!({"name": "x"});
        let cold = cmd(true, &[], &[]);
        let warm = cmd(false, &["godot_log_error"], &[]);
        let divergence = compare_equivalence(&scenario, &cold, &warm).expect("divergence");
        assert_eq!(divergence["cold_ok"], json!(true));
        assert_eq!(divergence["warm_ok"], json!(false));
        assert_eq!(
            divergence["failures_only_in_warm"],
            json!(["godot_log_error"])
        );
    }
