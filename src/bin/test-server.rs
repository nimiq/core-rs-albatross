extern crate tungstenite;

use std::net::TcpListener;
use std::thread::spawn;
use tungstenite::server::accept;

fn main() {
    // A WebSocket echo server
    let server = TcpListener::bind("127.0.0.1:8080").unwrap();
    for stream in server.incoming() {
        spawn(move || {
            let mut websocket = accept(stream.unwrap()).unwrap();
            const MSGS: &'static [&'static str] = &[
                // "4204204200000000e4ef6f9a59000000020000000000000000000000000000080808088f30a4d938d4130d1a3396dede1505c72b7f75ac9f9b80d1ad7e368b39f3b10500e5564300c360ac729086e2cc806e828a84877f1eb8e5d974d873e065224901555fb8821590a33bacc61e39701cf9b46bd25bf5f0595bbe24655141438e7a100bfafd3f1cc800309cb931081257b4e7edb0f9ea400187c550fd9141447ea086a5324dcf027dd4a30a932c441f365a25e86b173defa4b8e58948253471b81b72cf0000000000000000000000000000000000000000000000000000000000000000",
                "42042042010000007b268c0610000300000002324dcf027dd4a30a932c441f365a25e86b173defa4b8e58948253471b81b72cf00000002b8b37c1d034e371c7a3b834f9476a746eb62259ff9558ab715b4bff79ebf58e100000001f823f66ba1026e7f711ea5aa4719837bb378fc615b50516b8dabdaff78e8168e",
                "42042042020000007b990afcc2000300000002324dcf027dd4a30a932c441f365a25e86b173defa4b8e58948253471b81b72cf00000002b8b37c1d034e371c7a3b834f9476a746eb62259ff9558ab715b4bff79ebf58e100000001f823f66ba1026e7f711ea5aa4719837bb378fc615b50516b8dabdaff78e8168e",
                "4204204206000000ba2002774c0001000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000009d5b7130fdd19427406f1d477788ec1f866c650b2eff550afb3505b1435681bbbfacd81c643767f3bf1bf642c97b4efd33daeff2b0d341c613344ae706eedb381f010000000000010000000000018d5800011be440919634a6fe3ba5f8a7181fe4bb8212c13c0000000000",
                "42042042070000009fc8a9c2ed0001000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000009d5b7130fdd19427406f1d477788ec1f866c650b2eff550afb3505b1435681bbbfacd81c643767f3bf1bf642c97b4efd33daeff2b0d341c613344ae706eedb381f010000000000010000000000018d58",
                "42042042080000009803ad3340008f30a4d938d4130d1a3396dede1505c72b7f75ac9f9b80d1ad7e368b39f3b10591e9240f415223982edc345532630710e94a7f5200000000000022b8000000000000002a0000000004e5564300c360ac729086e2cc806e828a84877f1eb8e5d974d873e065224901555fb8821590a33bacc61e39701cf9b46bd25bf5f0595bbe24655141438e7a100b00",
                "420420420500000012f392d9600000000402",
                "42042042090000000d994373bd",
                "420420420a000000194422360c004104746573740003616263"
            ];
            let mut tag: u8 = 0;
            while MSGS.len() > tag as usize {
                let msg = format!("{:02x}{}", tag, &MSGS[tag as usize]);
                // println!("msg: {}", msg);
                let msg = hex_to_vec(msg);
                let msg = tungstenite::Message::binary(msg);
                websocket.write_message(msg).unwrap();
                tag += 1;
                let rx_msg = websocket.read_message().expect("Some problem");
                println!("Got message type: {:?}", rx_msg);
            }
            websocket.close(None).expect("Problem when closing websocket");
        });
    }
}

pub fn hex_to_vec(hex: String) -> Vec<u8> {

    // Make vector of bytes from octets
    let mut bytes = Vec::new();
    for i in 0..(hex.len()/2) {
        let res = u8::from_str_radix(&hex[2*i .. 2*i+2], 16);
        match res {
            Ok(v) => bytes.push(v),
            Err(e) => println!("Problem with hex: {}", e),
        };
    };

    bytes // now convert from Vec<u8> to b64-encoded String
}
