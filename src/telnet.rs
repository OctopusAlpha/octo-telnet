//! Telnet protocol parser and negotiator (RFC 854, RFC 855)
//! State machine that processes raw BBS byte streams, strips IAC sequences,
//! and generates negotiation responses.

#![allow(dead_code)]

pub const IAC: u8 = 0xFF;
pub const DONT: u8 = 0xFE;
pub const DO: u8 = 0xFD;
pub const WONT: u8 = 0xFC;
pub const WILL: u8 = 0xFB;
pub const SB: u8 = 0xFA;
pub const GA: u8 = 0xF9;
pub const EL: u8 = 0xF8;
pub const EC: u8 = 0xF7;
pub const AYT: u8 = 0xF6;
pub const AO: u8 = 0xF5;
pub const IP: u8 = 0xF4;
pub const BRK: u8 = 0xF3;
pub const DM: u8 = 0xF2;
pub const NOP: u8 = 0xF1;
pub const SE: u8 = 0xF0;
pub const EOR: u8 = 0xEF;

pub const OPT_BINARY: u8 = 0;
pub const OPT_ECHO: u8 = 1;
pub const OPT_SGA: u8 = 3;
pub const OPT_STATUS: u8 = 5;
pub const OPT_TIMING_MARK: u8 = 6;
pub const OPT_TTYPE: u8 = 24;
pub const OPT_NAWS: u8 = 31;
pub const OPT_TSPEED: u8 = 32;
pub const OPT_LFLOW: u8 = 33;
pub const OPT_LINEMODE: u8 = 34;
pub const OPT_ENVIRON: u8 = 36;
pub const OPT_NEW_ENVIRON: u8 = 39;
pub const OPT_CHARSET: u8 = 42;

pub const TTYPE_IS: u8 = 0;
pub const TTYPE_SEND: u8 = 1;
pub const NAWS_REQUEST: u8 = 0xFF;

const TERMINAL_TYPE: &[u8] = b"VT100";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Data,
    Iac,
    Will,
    Wont,
    Do,
    Dont,
    Sb,
    SbData(u8),
    SbIac(u8),
}

pub struct TelnetOutput {
    pub data: Vec<u8>,
    pub responses: Vec<u8>,
}

/// Stateful Telnet protocol parser
pub struct TelnetParser {
    state: State,
}

impl TelnetParser {
    pub fn new() -> Self {
        Self { state: State::Data }
    }

    /// Process a chunk of raw bytes from the BBS server.
    /// Returns clean data and negotiation responses to send back.
    pub fn process(&mut self, input: &[u8]) -> TelnetOutput {
        let mut data = Vec::new();
        let mut responses = Vec::new();

        for &byte in input {
            match self.state {
                State::Data => {
                    if byte == IAC {
                        self.state = State::Iac;
                    } else {
                        data.push(byte);
                    }
                }
                State::Iac => {
                    match byte {
                        IAC => { data.push(0xFF); self.state = State::Data; }
                        WILL => self.state = State::Will,
                        WONT => self.state = State::Wont,
                        DO => self.state = State::Do,
                        DONT => self.state = State::Dont,
                        SB => self.state = State::Sb,
                        GA | NOP | DM | BRK | IP | AO | AYT | EC | EL | EOR => {
                            self.state = State::Data;
                        }
                        _ => { self.state = State::Data; }
                    }
                }
                State::Will => {
                    self.handle_will(byte, &mut responses);
                    self.state = State::Data;
                }
                State::Wont => {
                    responses.extend_from_slice(&[IAC, DONT, byte]);
                    self.state = State::Data;
                }
                State::Do => {
                    self.handle_do(byte, &mut responses);
                    self.state = State::Data;
                }
                State::Dont => {
                    responses.extend_from_slice(&[IAC, WONT, byte]);
                    self.state = State::Data;
                }
                State::Sb => {
                    self.state = State::SbData(byte);
                }
                State::SbData(option) => {
                    if byte == IAC {
                        self.state = State::SbIac(option);
                    }
                }
                State::SbIac(option) => {
                    match byte {
                        SE => {
                            self.handle_subnegotiation(option, &mut responses);
                            self.state = State::Data;
                        }
                        IAC => { self.state = State::SbData(option); }
                        _ => { self.state = State::SbData(option); }
                    }
                }
            }
        }

        TelnetOutput { data, responses }
    }

    /// Accept ECHO, SGA, BINARY; refuse everything else
    fn handle_will(&self, option: u8, responses: &mut Vec<u8>) {
        match option {
            OPT_ECHO | OPT_SGA | OPT_BINARY => responses.extend_from_slice(&[IAC, DO, option]),
            _ => responses.extend_from_slice(&[IAC, DONT, option]),
        }
    }

    /// Accept TTYPE, NAWS, SGA; refuse everything else
    fn handle_do(&self, option: u8, responses: &mut Vec<u8>) {
        match option {
            OPT_TTYPE | OPT_NAWS | OPT_SGA => responses.extend_from_slice(&[IAC, WILL, option]),
            _ => responses.extend_from_slice(&[IAC, WONT, option]),
        }
    }

    /// Handle subnegotiation (e.g. terminal type query)
    fn handle_subnegotiation(&self, option: u8, responses: &mut Vec<u8>) {
        match option {
            OPT_TTYPE => {
                let mut resp = vec![IAC, SB, OPT_TTYPE, TTYPE_IS];
                resp.extend_from_slice(TERMINAL_TYPE);
                resp.extend_from_slice(&[IAC, SE]);
                responses.extend_from_slice(&resp);
            }
            _ => {}
        }
    }

    pub fn reset(&mut self) {
        self.state = State::Data;
    }
}

impl Default for TelnetParser {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_plain_data_passthrough() {
        let mut parser = TelnetParser::new();
        let output = parser.process(b"Hello, World!");
        assert_eq!(output.data, b"Hello, World!");
        assert!(output.responses.is_empty());
    }

    #[test]
    fn test_iac_escape() {
        let mut parser = TelnetParser::new();
        let output = parser.process(&[0x41, IAC, IAC, 0x42]);
        assert_eq!(output.data, &[0x41, 0xFF, 0x42]);
    }

    #[test]
    fn test_will_echo_negotiation() {
        let mut parser = TelnetParser::new();
        let output = parser.process(&[IAC, WILL, OPT_ECHO]);
        assert!(output.data.is_empty());
        assert_eq!(output.responses, &[IAC, DO, OPT_ECHO]);
    }

    #[test]
    fn test_do_ttype_negotiation() {
        let mut parser = TelnetParser::new();
        let output = parser.process(&[IAC, DO, OPT_TTYPE]);
        assert!(output.data.is_empty());
        assert_eq!(output.responses, &[IAC, WILL, OPT_TTYPE]);
    }

    #[test]
    fn test_subnegotiation_ttype() {
        let mut parser = TelnetParser::new();
        let output = parser.process(&[IAC, SB, OPT_TTYPE, TTYPE_SEND, IAC, SE]);
        assert!(output.data.is_empty());
        let mut expected = vec![IAC, SB, OPT_TTYPE, TTYPE_IS];
        expected.extend_from_slice(TERMINAL_TYPE);
        expected.extend_from_slice(&[IAC, SE]);
        assert_eq!(output.responses, expected);
    }

    #[test]
    fn test_mixed_data_and_commands() {
        let mut parser = TelnetParser::new();
        let mut input = Vec::new();
        input.extend_from_slice(b"Welcome");
        input.extend_from_slice(&[IAC, WILL, OPT_ECHO]);
        input.extend_from_slice(b" to BBS");
        let output = parser.process(&input);
        assert_eq!(output.data, b"Welcome to BBS");
        assert_eq!(output.responses, &[IAC, DO, OPT_ECHO]);
    }
}
