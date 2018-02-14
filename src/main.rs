/*
Copyright 2018 Tamas Blummer

Licensed under the Apache License, Version 2.0 (the "License");
you may not use this file except in compliance with the License.
You may obtain a copy of the License at

http://www.apache.org/licenses/LICENSE-2.0

Unless required by applicable law or agreed to in writing, software
distributed under the License is distributed on an "AS IS" BASIS,
WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
See the License for the specific language governing permissions and
limitations under the License.
*/

extern crate bitcoin;
extern crate num;

use num::FromPrimitive;
use bitcoin::blockdata::block::BlockHeader;
use bitcoin::blockdata::constants::{genesis_block,DIFFCHANGE_INTERVAL,DIFFCHANGE_TIMESPAN,max_target};
use bitcoin::network::serialize::BitcoinHash;
use bitcoin::network::constants::Network;
use bitcoin::network::message_blockdata;
use bitcoin::network::socket::Socket;
use bitcoin::util;
use bitcoin::util::hash::Sha256dHash;
use bitcoin::util::uint::Uint256;
use bitcoin::util::BitArray;
use bitcoin::network::message::NetworkMessage::*;

struct HeaderWithDifficulty {
    pub header : BlockHeader,
    pub difficulty: u64,
    pub implied_difficulty: u64
}

fn main() {
    let mut headers : Vec<BlockHeader> = vec!();
    headers.push(genesis_block(Network::Bitcoin).header);
    match download_header(&mut headers) {
        Ok(()) => {
            let headers_with_difficulty = compute_difficulty(&headers).unwrap();
            let mut n = 0;
            for header in headers_with_difficulty  {
                println!("{}, {}, {}, {}", n, header.header.time, header.difficulty, header.implied_difficulty);
                n += 1;
            }
        }
        Err(e) => println!("{}", e)
    }

}

const SCALE_PRECISION: u32 = 1000;

fn single_interval (headers: &Vec<BlockHeader>, sdif :u64, i :usize) -> i32 {
    let scale: u32  = (sdif / headers[i].difficulty(Network::Bitcoin)) as u32;
    if headers [i+1].time < headers [i].time {
        -((((headers [i].time - headers [i+1].time) * scale)/SCALE_PRECISION) as i32)
    } else {
        (((headers[i+1].time - headers[i].time) * scale)/SCALE_PRECISION) as i32
    }
}

fn compute_difficulty(headers: &Vec<BlockHeader>) ->Result<Vec<HeaderWithDifficulty>, util::Error> {
    let mut headers_with : Vec<HeaderWithDifficulty> = vec!();

    let mut height: usize = 0;
    let max_target = max_target(Network::Bitcoin);
    for i in 0 as usize .. DIFFCHANGE_INTERVAL as usize + 1 {
        headers_with.push(HeaderWithDifficulty{
           header: headers[i],
            difficulty: 1,
            implied_difficulty: 1
        });
    }
    for header in headers {
        if height % 10000 == 0 {
            eprintln!("{}", height);
        }
        if height > DIFFCHANGE_INTERVAL as usize {
            // this is the original Bitcoin time interval computation
            // that below loop converges to at adjustment points, but provides low volatility estimates inbetween.
            //let interval2 = (headers[height - 1].time - headers[height - DIFFCHANGE_INTERVAL as usize].time) as i32;
            let mut interval = 0;
            let sdif = headers[height - DIFFCHANGE_INTERVAL as usize].difficulty(Network::Bitcoin) * SCALE_PRECISION as u64;
            for i in height - DIFFCHANGE_INTERVAL as usize .. height - 1 as usize {
                interval += single_interval(headers, sdif, i);
            }
            let adjusted_interval = match interval as u32 {
                n if n < DIFFCHANGE_TIMESPAN / 4 => DIFFCHANGE_TIMESPAN / 4,
                n if n > DIFFCHANGE_TIMESPAN * 4 => DIFFCHANGE_TIMESPAN * 4,
                n => n
            };
            let mut target = headers[height - 1].target();
            target = target.mul_u32(adjusted_interval);
            target = target / FromPrimitive::from_u64(DIFFCHANGE_TIMESPAN as u64).unwrap();
            if target > max_target { target = max_target };
            target = satoshi_the_precision(target);
            if height as u32 % DIFFCHANGE_INTERVAL == 0 && header.target() != target {
                return Err(util::Error::SpvBadTarget);
            }

            let mut implied_target = headers[height - DIFFCHANGE_INTERVAL as usize].target();
            implied_target = implied_target.mul_u32(adjusted_interval);
            implied_target = implied_target / FromPrimitive::from_u64(DIFFCHANGE_TIMESPAN as u64).unwrap();
            if implied_target > max_target { implied_target = max_target };
            implied_target = satoshi_the_precision(implied_target);


            headers_with.push(HeaderWithDifficulty{
               header: headers [height],
                difficulty : header.difficulty(Network::Bitcoin),
                implied_difficulty : (max_target / implied_target).low_u64()
            });
        }
        height += 1;
    }
    Ok(headers_with)
}

fn download_header(headers: &mut Vec<BlockHeader>) -> Result<(), util::Error> {
    let mut socket = Socket::new(Network::Bitcoin);
    socket.connect("127.0.0.1", 8333)?;
    let version_message = socket.version_message(0)?;
    socket.send_message(version_message)?;
    let mut highest :usize = 0;
    let mut read = 0;
    loop {
        // Receive new message
        match socket.receive_message() {
            Ok(payload) => {
                match payload {
                    Verack => {},
                    Version(m) => {
                        highest = m.start_height as usize;
                        socket.send_message(Verack)?;
                        continue_header_download(&mut socket, headers)?;
                    }
                    Headers(vec) => {
                        for header in &vec {
                            headers.push(header.header);
                        }
                        read += vec.len();
                        if read < highest {
                            continue_header_download(&mut socket, headers)?;
                        }
                        else {
                            return Ok(())
                        }
                    }
                    Ping(n) => socket.send_message(Pong(n))?,
                    _ => {}
                }
            }
            Err(e) => {
                return Err(e)
            }
        }
    }
}

fn continue_header_download(socket: &mut Socket, headers: &mut Vec<BlockHeader>) -> Result<(), util::Error> {
    let mut locator: Vec<Sha256dHash> = vec!();
    locator.push(headers.last().unwrap().bitcoin_hash());
    socket.send_message(GetHeaders(
        message_blockdata::GetHeadersMessage::new(locator, Sha256dHash::default())))
}

// below this line is a copy from:
// ----------------------------------------
// Written in 2014 by
//     Andrew Poelstra <apoelstra@wpsoftware.net>
//
// To the extent possible under law, the author(s) have dedicated all
// copyright and related and neighboring rights to this software to
// the public domain worldwide. This software is distributed without
// any warranty.
//
// You should have received a copy of the CC0 Public Domain Dedication
// along with this software.
// If not, see <http://creativecommons.org/publicdomain/zero/1.0/>.
//
/// This function emulates the `GetCompact(SetCompact(n))` in the satoshi code,
/// which drops the precision to something that can be encoded precisely in
/// the nBits block header field. Savour the perversity. This is in Bitcoin
/// consensus code. What. Gaah!
fn satoshi_the_precision(n: Uint256) -> Uint256 {
    // Shift by B bits right then left to turn the low bits to zero
    let bits = 8 * ((n.bits() + 7) / 8 - 3);
    let mut ret = n >> bits;
    // Oh, did I say B was that fucked up formula? I meant sometimes also + 8.
    if ret.bit(23) {
        ret = (ret >> 8) << 8;
    }
    ret << bits
}


