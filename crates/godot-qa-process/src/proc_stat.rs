/// Fields beginning with state (field 3), following the process comm.
pub(crate) fn fields(stat: &[u8]) -> Option<std::str::SplitWhitespace<'_>> {
    let end = stat.iter().rposition(|byte| *byte == b')')?;
    Some(
        std::str::from_utf8(stat.get(end + 1..)?)
            .ok()?
            .split_whitespace(),
    )
}

#[cfg(test)]
mod tests {
    #[test]
    fn parses_non_utf8_comm_without_decoding_it() {
        let stat =
            b"123 (bad\xff na)me) S 1 456 6 7 8 9 10 11 12 13 14 15 16 17 18 19 20 21 987654";
        let fields: Vec<_> = super::fields(stat)
            .expect("ASCII fields after arbitrary comm")
            .collect();
        assert_eq!(fields[0], "S");
        assert_eq!(fields[2], "456");
        assert_eq!(fields[19], "987654");
    }

    #[test]
    fn parses_state_group_and_start_time_after_spaced_parenthesised_comm() {
        let stat =
            "123 (worker (qa) name)) Z 1 456 6 7 8 9 10 11 12 13 14 15 16 17 18 19 20 21 987654";
        let fields: Vec<_> = super::fields(stat.as_bytes()).unwrap().collect();
        assert_eq!(fields[0], "Z");
        assert_eq!(fields[2], "456");
        assert_eq!(fields[19], "987654");
    }
}
