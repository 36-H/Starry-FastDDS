use alloc::sync::Arc;
use arceos_api::display;
use axsync::Mutex;
use core::fmt::{self, write, Display};
use crate::syscall_fs::PipeRingBuffer;
use alloc::vec::Vec;
use axtask::yield_now;
use axerrno::{AxError, AxResult};
use axnet::{SocketAddr, IpAddr};
use axlog::error;


use super::imp;
/**
 * nlmsghdr - netlink message header
 * @nlmsg_len: 紧跟该结构的数据部分长度以及该结构的大小
 * @nlmsg_type: 应用内部定义消息的类型，对netlink内核实现是透明的，因此大部分情况下设置为0
 * @nlmsg_flags: 设置消息标志
 * @nlmsg_seq: 消息序列号，用以将消息排队,可选
 * @nlmsg_pid: 消息来源进程 ID
 * 
 * /// struct nlmsghdr {
 * ///     __u32 nlmsg_len;   
 * ///     __u16 nlmsg_type;
 * ///     __u16 nlmsg_flags;
 * ///     __u32 nlmsg_seq;
 * ///     __u32 nlmsg_pid;
 */
pub struct NetlinkMessageHeader {
    len: u32,
    message_type: u16,
    flags: u16,
    sequence_number: u32,
    port_id: u32,
}

impl NetlinkMessageHeader {
    pub fn new(len: u32, message_type: u16, flags: u16, sequence_number: u32, port_id: u32) -> Self {
        NetlinkMessageHeader {
            len,
            message_type,
            flags,
            sequence_number,
            port_id,
        }
    }

    pub fn create_from_buffer(buf: &[u8]) -> Self{
        let len = u32::from_ne_bytes(buf[0..4].try_into().unwrap());
        let message_type = u16::from_ne_bytes(buf[4..6].try_into().unwrap());
        let flags = u16::from_ne_bytes(buf[6..8].try_into().unwrap());
        let sequence_number = u32::from_ne_bytes(buf[8..12].try_into().unwrap());
        let port_id = u32::from_ne_bytes(buf[12..16].try_into().unwrap());
        NetlinkMessageHeader {
            len,
            message_type,
            flags,
            sequence_number,
            port_id,
        }
    }    
}

impl Display for NetlinkMessageHeader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "NetlinkMessageHeader {{ len: {}, message_type: {}, flags: {}, sequence_number: {}, port_id: {} }}",
            self.len, self.message_type, self.flags, self.sequence_number, self.port_id
        )
    }
}


// struct sockaddr_nl
//{
//    sa_family_t nl_family; /*该字段总是为AF_NETLINK */
//    unsigned short nl_pad; /* 目前未用到，填充为0*/
//    __u32 nl_pid; /* process pid */
//    __u32 nl_groups; /* multicast groups mask */
//}
/**
* nl_pid 在Netlink规范里，全称为Port-ID 32bits 
* 其主要作用是用于唯一的标识一个基于netlink的socket通道。
* 通常情况下nl_pid都设置为当前进程的进程号,为0时一般表示为内核。  
*
* nl_group 该字段指明了调用者希望加入的多播组号的掩码(注意不是组号)。
* 如果该字段为0则表示调用者不希望加入任何多播组。
*/
// struct SocketAddressNetLink {
//     nl_family: u16,
//     nl_pad: u16,
//     nl_pid : u32,
//     nl_groups: u32
// }

// impl SocketAddressNetLink {
//     pub fn new(nl_family: u16, nl_pad: u16, nl_pid: u32, nl_groups: u32) -> Self {
//         SocketAddressNetLink {
//             nl_family,
//             nl_pad,
//             nl_pid,
//             nl_groups,
//         }
//     }
// }

// struct Iovec{
//     iov_len: usize,
//     iov_base: Arc<Mutex<PipeRingBuffer>>
// }

// impl Iovec {
//     pub fn new(iov_len: usize, iov_base: Arc<Mutex<PipeRingBuffer>>) -> Self {
//         Iovec {
//             iov_len,
//             iov_base,
//         }
//     }
// }

// pub struct NetlinkSocket{
//     msg_name: SocketAddressNetLink,
//     msg_len: u32,
//     msg_iov: Vec<Iovec>,
//     msg_control: Arc<Mutex<PipeRingBuffer>>,
//     msg_controllen: usize,
//     msg_flags: i32
// }

// impl NetlinkSocket {
//     pub fn new(msg_name: SocketAddressNetLink, msg_len: u32, msg_iov: Vec<Iovec>, msg_control: Arc<Mutex<PipeRingBuffer>>, msg_controllen: usize, msg_flags: i32) -> Self {
//         NetlinkSocket {
//             msg_name,
//             msg_len,
//             msg_iov,
//             msg_control,
//             msg_controllen,
//             msg_flags
//         }
//     }
// }


pub struct NetlinkSocket {
    pid: u64,
    group: u32,
    buffer: Arc<Mutex<PipeRingBuffer>>,
} 

impl NetlinkSocket {
    pub fn new(pid: u64, group: u32) -> Self {
        Self {
            pid,
            group,
            buffer: Arc::new(Mutex::new(PipeRingBuffer::new())),
        }
    }

    pub fn write(&self, buf: &[u8]) -> AxResult<usize> {
        let want_to_write = buf.len();
        let mut buf_iter = buf.iter();
        let mut already_write = 0usize;
        loop {
            let mut ring_buffer = self.buffer.lock();
            let loop_write = ring_buffer.available_write();
            if loop_write == 0 {
                drop(ring_buffer);
                yield_now();
                continue;
            }
            // write at most loop_write bytes
            for _ in 0..loop_write {
                if let Some(byte_ref) = buf_iter.next() {
                    ring_buffer.write_byte(*byte_ref);
                    already_write += 1;
                    if already_write == want_to_write {
                        drop(ring_buffer);
                        return Ok(want_to_write);
                    }
                } else {
                    break;
                }
            }
            return Ok(already_write);
        }
    }

    pub fn send(&self, data: &[u8]) -> AxResult<usize> {
        let nlh = NetlinkMessageHeader::create_from_buffer(data);
        error!("NetlinkSocket::send: {}", nlh);
        match nlh.message_type {
            18 => {
                // \24\0\0\0\3\0\2\0\2\0\0\0,\0\0\0\0\0\0\0
                let buf: [u8; 20] = [0x24, 0x00, 0x00, 0x00, 0x03, 0x00, 0x02, 0x00, 0x02, 0x00, 0x00, 0x00, 0x2c, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00];
                self.write(&buf);
                Ok(20)
            },
            22 => {
                // \24\0\0\0\3\0\2\0\1\0\0\0,\0\0\0\0\0\0\0
                let buf: [u8; 20] = [0x24, 0x00, 0x00, 0x00, 0x03, 0x00, 0x02, 0x00, 0x01, 0x00, 0x00, 0x00, 0x2c, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00];
                self.write(&buf);
                Ok(20)
            },
            _ => {
                error!("NetlinkSocket::send: unknown type: {}", nlh.message_type);
                Err(AxError::Unsupported)
            }
        }
    }

    pub fn read(&self, buf: &mut [u8]) -> AxResult<usize> {
        let want_to_read = buf.len();
        let mut buf_iter = buf.iter_mut();
        let mut already_read = 0usize;
        loop {
            let mut ring_buffer = self.buffer.lock();
            let loop_read = ring_buffer.available_read();
            if loop_read == 0 {
                drop(ring_buffer);
                yield_now();
                continue;
            }
            // read at most loop_read bytes
            for _ in 0..loop_read {
                if let Some(byte_ref) = buf_iter.next() {
                    *byte_ref = ring_buffer.read_byte();
                    already_read += 1;
                    if already_read == want_to_read {
                        drop(ring_buffer);
                        return Ok(want_to_read);
                    }
                } else {
                    break;
                }
            }
            return Ok(already_read);
        }
    }

    pub fn get_peer_addr(&self) -> SocketAddr {
        let raw = IpAddr::v4(0, 0, 0, 0);
        SocketAddr::new(raw, 0)
    }

}