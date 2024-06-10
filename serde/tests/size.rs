mod derive_exact {
    use nimiq_serde::SerializedSize;

    struct Size123;
    struct Size456;

    impl SerializedSize for Size123 {
        const SIZE: usize = 123;
    }
    impl SerializedSize for Size456 {
        const SIZE: usize = 456;
    }

    #[test]
    fn empty_struct() {
        #[allow(dead_code)]
        #[derive(SerializedSize)]
        struct Empty;

        assert_eq!(Empty::SIZE, 0);
    }

    #[test]
    fn struct_() {
        #[allow(dead_code)]
        #[derive(SerializedSize)]
        struct Struct(Size123, Size456);

        assert_eq!(Struct::SIZE, 123 + 456);
    }

    #[test]
    fn array() {
        #[allow(dead_code)]
        #[derive(SerializedSize)]
        struct Struct([Size123; 456]);

        assert_eq!(Struct::SIZE, 123 * 456);
    }
}

mod derive_max {
    use nimiq_collections::BitSet;
    use nimiq_serde::{seq_max_size, SerializedMaxSize, SerializedSize as _};

    struct Size123;
    struct Size456;

    impl SerializedMaxSize for Size123 {
        const MAX_SIZE: usize = 123;
    }
    impl SerializedMaxSize for Size456 {
        const MAX_SIZE: usize = 456;
    }

    #[test]
    fn empty_struct() {
        #[allow(dead_code)]
        #[derive(SerializedMaxSize)]
        struct Empty;

        assert_eq!(Empty::MAX_SIZE, 0);
    }

    #[test]
    fn vec() {
        #[allow(dead_code)]
        #[derive(SerializedMaxSize)]
        struct Max32Bytes {
            #[serialize_size(seq_max_elems = 32)]
            bytes: Vec<u8>,
        }

        assert_eq!(Max32Bytes::MAX_SIZE, seq_max_size(u8::SIZE, 32));
    }

    #[test]
    fn option() {
        assert_eq!(Option::<Size123>::MAX_SIZE, 1 + 123);
    }

    #[test]
    fn bitset() {
        #[allow(dead_code)]
        #[derive(SerializedMaxSize)]
        struct Wrapper {
            #[serialize_size(bitset_max_elem = 123456)]
            bitset: BitSet,
        }

        assert_eq!(Wrapper::MAX_SIZE, BitSet::max_size(123456));
    }

    #[test]
    fn struct_() {
        #[allow(dead_code)]
        #[derive(SerializedMaxSize)]
        struct Struct(Size123, Size456);

        assert_eq!(Struct::MAX_SIZE, 123 + 456);
    }

    #[test]
    fn enum_() {
        #[allow(dead_code)]
        #[derive(SerializedMaxSize)]
        enum Enum {
            First(Size123),
            Second(Size456),
        }

        assert_eq!(Enum::MAX_SIZE, 1 + 456);
    }
}
