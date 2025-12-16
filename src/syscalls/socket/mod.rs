#![allow(dead_code)]
#![allow(nonstandard_style)]

mod addrinfo;

use alloc::boxed::Box;
use alloc::sync::Arc;
use core::ffi::{c_char, c_void};
use core::mem::{self, size_of};
use core::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV4, SocketAddrV6};
#[allow(unused_imports)]
use core::ops::DerefMut;

use num_enum::{IntoPrimitive, TryFromPrimitive, TryFromPrimitiveError};

use crate::errno::Errno;
#[cfg(feature = "vsock")]
use crate::fd::socket::vsock::{self, VsockEndpoint, VsockListenEndpoint};
use crate::fd::{
	self, Endpoint, ListenEndpoint, ObjectInterface, SocketOption, get_object, insert_object,
};
use crate::syscalls::block_on;

#[derive(TryFromPrimitive, IntoPrimitive, PartialEq, Eq, Clone, Copy, Debug)]
#[repr(u8)]
pub enum Af {
	Unspec = 0,
	Inet = 3,
	Inet6 = 1,
	Unix = 4,
	#[cfg(feature = "vsock")]
	Vsock = 2,
}

impl From<IpAddr> for Af {
	fn from(value: IpAddr) -> Self {
		match value {
			IpAddr::V4(_) => Self::Inet,
			IpAddr::V6(_) => Self::Inet6,
		}
	}
}

#[derive(TryFromPrimitive, IntoPrimitive, PartialEq, Eq, Clone, Copy, Debug)]
#[repr(u8)]
pub enum Ipproto {
	Ip = 0,
	Ipv6 = 41,
	Tcp = 6,
	Udp = 17,
}

pub const IPV6_ADD_MEMBERSHIP: i32 = 12;
pub const IPV6_DROP_MEMBERSHIP: i32 = 13;
pub const IPV6_MULTICAST_LOOP: i32 = 19;
pub const IPV6_V6ONLY: i32 = 27;
pub const IP_TOS: i32 = 1;
pub const IP_TTL: i32 = 2;
pub const IP_MULTICAST_TTL: i32 = 5;
pub const IP_MULTICAST_LOOP: i32 = 7;
pub const IP_ADD_MEMBERSHIP: i32 = 3;
pub const IP_DROP_MEMBERSHIP: i32 = 4;
pub const SOL_SOCKET: i32 = 4095;
pub const SO_REUSEADDR: i32 = 0x0004;
pub const SO_KEEPALIVE: i32 = 0x0008;
pub const SO_BROADCAST: i32 = 0x0020;
pub const SO_LINGER: i32 = 0x0080;
pub const SO_SNDBUF: i32 = 0x1001;
pub const SO_RCVBUF: i32 = 0x1002;
pub const SO_SNDTIMEO: i32 = 0x1005;
pub const SO_RCVTIMEO: i32 = 0x1006;
pub const SO_ERROR: i32 = 0x1007;
pub const TCP_NODELAY: i32 = 1;
pub const MSG_PEEK: i32 = 1;
pub type sa_family_t = u8;
pub type socklen_t = u32;
pub type in_addr_t = u32;
pub type in_port_t = u16;

#[derive(TryFromPrimitive, IntoPrimitive, PartialEq, Eq, Clone, Copy, Debug)]
#[repr(u8)]
pub enum Sock {
	Stream = 1,
	Dgram = 2,
	Raw = 3,
	Seqpacket = 5,
}

bitflags! {
	#[derive(Debug, Copy, Clone)]
	#[repr(C)]
	pub struct SockFlags: i32 {
		const SOCK_NONBLOCK = 0o4000;
		const SOCK_CLOEXEC = 0o40000;
		const _ = !0;
	}
}

impl Sock {
	pub fn from_bits(bits: i32) -> Option<(Self, SockFlags)> {
		let sock = Sock::try_from(bits as u8).ok()?;
		let flags = SockFlags::from_bits_retain(bits & !0xff);
		Some((sock, flags))
	}
}

#[repr(C)]
#[derive(Debug, Default, Copy, Clone)]
pub struct in_addr {
	pub s_addr: in_addr_t,
}

impl From<Ipv4Addr> for in_addr {
	fn from(value: Ipv4Addr) -> Self {
		Self {
			s_addr: u32::from_ne_bytes(value.octets()),
		}
	}
}

#[repr(C, align(4))]
#[derive(Debug, Default, Copy, Clone)]
pub struct in6_addr {
	pub s6_addr: [u8; 16],
}

impl From<Ipv6Addr> for in6_addr {
	fn from(value: Ipv6Addr) -> Self {
		Self {
			s6_addr: value.octets(),
		}
	}
}

#[repr(C)]
#[derive(Debug, Default, Copy, Clone)]
pub struct sockaddr {
	pub sa_len: u8,
	pub sa_family: sa_family_t,
	pub sa_data: [c_char; 14],
}

#[derive(Clone, Debug)]
pub enum sockaddrBox {
	sockaddr(Box<sockaddr>),
	sockaddr_in(Box<sockaddr_in>),
	sockaddr_in6(Box<sockaddr_in6>),
	sockaddr_un(Box<sockaddr_un>),
	#[cfg(feature = "vsock")]
	sockaddr_vm(Box<sockaddr_vm>),
}

#[derive(Clone, Copy, Debug)]
pub enum sockaddrRef<'a> {
	sockaddr(&'a sockaddr),
	sockaddr_in(&'a sockaddr_in),
	sockaddr_in6(&'a sockaddr_in6),
	sockaddr_un(&'a sockaddr_un),
	#[cfg(feature = "vsock")]
	sockaddr_vm(&'a sockaddr_vm),
}

impl sockaddr {
	pub unsafe fn sa_family(ptr: *const Self) -> Result<Af, TryFromPrimitiveError<Af>> {
		let sa_family = unsafe { (*ptr).sa_family };
		Af::try_from(sa_family)
	}

	pub unsafe fn as_ref(ptr: &*const Self) -> Result<sockaddrRef<'_>, TryFromPrimitiveError<Af>> {
		let ptr = *ptr;
		let sa_family = unsafe { Self::sa_family(ptr)? };
		let ret = match sa_family {
			Af::Unspec => sockaddrRef::sockaddr(unsafe { &*ptr }),
			Af::Inet => sockaddrRef::sockaddr_in(unsafe { &*ptr.cast() }),
			Af::Inet6 => sockaddrRef::sockaddr_in6(unsafe { &*ptr.cast() }),
			Af::Unix => sockaddrRef::sockaddr_un(unsafe { &*ptr.cast() }),
			#[cfg(feature = "vsock")]
			Af::Vsock => sockaddrRef::sockaddr_vm(unsafe { &*ptr.cast() }),
		};
		Ok(ret)
	}

	pub unsafe fn as_box(ptr: *mut Self) -> Result<sockaddrBox, TryFromPrimitiveError<Af>> {
		let sa_family = unsafe { Self::sa_family(ptr)? };
		let ret = match sa_family {
			Af::Unspec => sockaddrBox::sockaddr(unsafe { Box::from_raw(ptr) }),
			Af::Inet => sockaddrBox::sockaddr_in(unsafe { Box::from_raw(ptr.cast()) }),
			Af::Inet6 => sockaddrBox::sockaddr_in6(unsafe { Box::from_raw(ptr.cast()) }),
			Af::Unix => sockaddrBox::sockaddr_un(unsafe { Box::from_raw(ptr.cast()) }),
			#[cfg(feature = "vsock")]
			Af::Vsock => sockaddrBox::sockaddr_vm(unsafe { Box::from_raw(ptr.cast()) }),
		};
		Ok(ret)
	}
}

impl sockaddrBox {
	pub fn into_raw(self) -> *mut sockaddr {
		match self {
			sockaddrBox::sockaddr(sockaddr) => Box::into_raw(sockaddr),
			sockaddrBox::sockaddr_in(sockaddr_in) => Box::into_raw(sockaddr_in).cast(),
			sockaddrBox::sockaddr_in6(sockaddr_in6) => Box::into_raw(sockaddr_in6).cast(),
			sockaddrBox::sockaddr_un(sockaddr_un) => Box::into_raw(sockaddr_un).cast(),
			#[cfg(feature = "vsock")]
			sockaddrBox::sockaddr_vm(sockaddr_vm) => Box::into_raw(sockaddr_vm).cast(),
		}
	}

	pub fn as_ref(&self) -> sockaddrRef<'_> {
		match self {
			Self::sockaddr(sockaddr) => sockaddrRef::sockaddr(sockaddr.as_ref()),
			Self::sockaddr_in(sockaddr_in) => sockaddrRef::sockaddr_in(sockaddr_in.as_ref()),
			Self::sockaddr_in6(sockaddr_in6) => sockaddrRef::sockaddr_in6(sockaddr_in6.as_ref()),
			Self::sockaddr_un(sockaddr_un) => sockaddrRef::sockaddr_un(sockaddr_un.as_ref()),
			#[cfg(feature = "vsock")]
			Self::sockaddr_vm(sockaddr_vm) => sockaddrRef::sockaddr_vm(sockaddr_vm.as_ref()),
		}
	}
}

impl From<SocketAddr> for sockaddrBox {
	fn from(value: SocketAddr) -> Self {
		match value {
			SocketAddr::V4(socket_addr_v4) => Self::sockaddr_in(Box::new(socket_addr_v4.into())),
			SocketAddr::V6(socket_addr_v6) => Self::sockaddr_in6(Box::new(socket_addr_v6.into())),
		}
	}
}

impl sockaddrRef<'_> {
	pub fn addrlen(self) -> u8 {
		match self {
			sockaddrRef::sockaddr(sockaddr) => sockaddr.sa_len,
			sockaddrRef::sockaddr_in(sockaddr_in) => sockaddr_in.sin_len,
			sockaddrRef::sockaddr_in6(sockaddr_in6) => sockaddr_in6.sin6_len,
			sockaddrRef::sockaddr_un(sockaddr_un) => sockaddr_un.sun_len,
			#[cfg(feature = "vsock")]
			sockaddrRef::sockaddr_vm(sockaddr_vm) => sockaddr_vm.svm_len,
		}
	}
}

#[cfg(feature = "vsock")]
#[repr(C)]
#[derive(Debug, Copy, Clone, Default)]
pub struct sockaddr_vm {
	pub svm_len: u8,
	pub svm_family: sa_family_t,
	pub svm_reserved1: u16,
	pub svm_port: u32,
	pub svm_cid: u32,
	pub svm_zero: [u8; 4],
}

#[cfg(feature = "vsock")]
impl From<sockaddr_vm> for VsockListenEndpoint {
	fn from(addr: sockaddr_vm) -> VsockListenEndpoint {
		let port = addr.svm_port;
		let cid = if addr.svm_cid < u32::MAX {
			Some(addr.svm_cid)
		} else {
			None
		};

		VsockListenEndpoint::new(port, cid)
	}
}

#[cfg(feature = "vsock")]
impl From<sockaddr_vm> for VsockEndpoint {
	fn from(addr: sockaddr_vm) -> VsockEndpoint {
		let port = addr.svm_port;
		let cid = addr.svm_cid;

		VsockEndpoint::new(port, cid)
	}
}

#[cfg(feature = "vsock")]
impl From<VsockEndpoint> for sockaddr_vm {
	fn from(endpoint: VsockEndpoint) -> Self {
		Self {
			svm_len: core::mem::size_of::<sockaddr_vm>().try_into().unwrap(),
			svm_family: Af::Vsock.into(),
			svm_port: endpoint.port,
			svm_cid: endpoint.cid,
			..Default::default()
		}
	}
}

#[repr(C)]
#[derive(Debug, Default, Copy, Clone)]
pub struct sockaddr_in {
	pub sin_len: u8,
	pub sin_family: sa_family_t,
	pub sin_port: in_port_t,
	pub sin_addr: in_addr,
	pub sin_zero: [c_char; 8],
}

impl From<SocketAddrV4> for sockaddr_in {
	fn from(value: SocketAddrV4) -> Self {
		Self {
			sin_len: mem::size_of::<Self>().try_into().unwrap(),
			sin_family: Af::Inet.into(),
			sin_port: value.port().to_be(),
			sin_addr: (*value.ip()).into(),
			sin_zero: Default::default(),
		}
	}
}

#[repr(C)]
#[derive(Debug, Default, Copy, Clone)]
pub struct sockaddr_in6 {
	pub sin6_len: u8,
	pub sin6_family: sa_family_t,
	pub sin6_port: in_port_t,
	pub sin6_flowinfo: u32,
	pub sin6_addr: in6_addr,
	pub sin6_scope_id: u32,
}

impl From<SocketAddrV6> for sockaddr_in6 {
	fn from(value: SocketAddrV6) -> Self {
		Self {
			sin6_len: mem::size_of::<Self>().try_into().unwrap(),
			sin6_family: Af::Inet6.into(),
			sin6_port: value.port().to_be(),
			sin6_flowinfo: Default::default(),
			sin6_addr: (*value.ip()).into(),
			sin6_scope_id: Default::default(),
		}
	}
}

#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct sockaddr_un {
	pub sun_len: u8,
	pub sun_family: sa_family_t,
	pub sun_path: [c_char; 104],
}

#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct ip_mreq {
	pub imr_multiaddr: in_addr,
	pub imr_interface: in_addr,
}

#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct ipv6_mreq {
	pub ipv6mr_multiaddr: in6_addr,
	pub ipv6mr_interface: u32,
}
#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct linger {
	pub l_onoff: i32,
	pub l_linger: i32,
}

#[hermit_macro::system(errno)]
#[unsafe(no_mangle)]
pub extern "C" fn sys_socket(domain: i32, type_: i32, protocol: i32) -> i32 {
	debug!("sys_socket: domain {domain}, type {type_:?}, protocol {protocol}");

	let Ok(Ok(domain)) = u8::try_from(domain).map(Af::try_from) else {
		return -i32::from(Errno::Afnosupport);
	};

	let Some((sock, sock_flags)) = Sock::from_bits(type_) else {
		return -i32::from(Errno::Socktnosupport);
	};

	// We do not support the exec syscall, so SOCK_CLOEXEC does not need an implementation.
	let supported_flags = SockFlags::SOCK_NONBLOCK | SockFlags::SOCK_CLOEXEC;
	if !(sock_flags - supported_flags).is_empty() {
		return -i32::from(Errno::Inval);
	}

	let Ok(Ok(proto)) = u8::try_from(protocol).map(Ipproto::try_from) else {
		return -i32::from(Errno::Protonosupport);
	};

	match (sock, proto) {
		(_, Ipproto::Ip | Ipproto::Ipv6)
		| (Sock::Stream, Ipproto::Tcp)
		| (Sock::Dgram, Ipproto::Udp) => {}
		(_, _) => return -i32::from(Errno::Prototype),
	}

	#[cfg(feature = "vsock")]
	if domain == Af::Vsock {
		if sock != Sock::Stream {
			return -i32::from(Errno::Socktnosupport);
		}

		let mut socket = vsock::Socket::new();

		if sock_flags.contains(SockFlags::SOCK_NONBLOCK) {
			block_on(socket.set_status_flags(fd::StatusFlags::O_NONBLOCK), None).unwrap();
		}

		let socket = Arc::new(async_lock::RwLock::new(socket));
		let fd = insert_object(socket).expect("FD is already used");

		return fd;
	}

	// If we still haven't returned, it means that the domain is not supported.
	-i32::from(Errno::Afnosupport)
}

#[hermit_macro::system(errno)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sys_accept(fd: i32, addr: *mut sockaddr, addrlen: *mut socklen_t) -> i32 {
	let obj = get_object(fd);
	obj.map_or_else(
		|e| -i32::from(e),
		|v| {
			block_on(async { v.write().await.accept().await }, None).map_or_else(
				|e| -i32::from(e),
				|endpoint| match endpoint {
					#[cfg(feature = "vsock")]
					Endpoint::Vsock(endpoint) => {
						let new_fd = insert_object(v.clone()).unwrap();

						if !addr.is_null() && !addrlen.is_null() {
							let addrlen = unsafe { &mut *addrlen };

							if *addrlen >= u32::try_from(size_of::<sockaddr_vm>()).unwrap() {
								let addr = unsafe { &mut *addr.cast() };
								*addr = sockaddr_vm::from(endpoint);
								*addrlen = size_of::<sockaddr_vm>().try_into().unwrap();
							}
						}

						new_fd
					}
				},
			)
		},
	)
}

#[hermit_macro::system(errno)]
#[unsafe(no_mangle)]
pub extern "C" fn sys_listen(fd: i32, backlog: i32) -> i32 {
	let obj = get_object(fd);
	obj.map_or_else(
		|e| -i32::from(e),
		|v| {
			block_on(async { v.write().await.listen(backlog).await }, None)
				.map_or_else(|e| -i32::from(e), |()| 0)
		},
	)
}

#[hermit_macro::system(errno)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sys_bind(fd: i32, name: *const sockaddr, namelen: socklen_t) -> i32 {
	if name.is_null() {
		return -i32::from(Errno::Destaddrreq);
	}

	let Ok(family) = (unsafe { Af::try_from((*name).sa_family) }) else {
		return -i32::from(Errno::Inval);
	};

	let obj = get_object(fd);
	obj.map_or_else(
		|e| -i32::from(e),
		|v| match family {
			#[cfg(feature = "vsock")]
			Af::Vsock => {
				if namelen < u32::try_from(size_of::<sockaddr_vm>()).unwrap() {
					return -i32::from(Errno::Inval);
				}
				let endpoint = VsockListenEndpoint::from(unsafe { *name.cast::<sockaddr_vm>() });
				block_on(
					async { v.write().await.bind(ListenEndpoint::Vsock(endpoint)).await },
					None,
				)
				.map_or_else(|e| -i32::from(e), |()| 0)
			}
			_ => -i32::from(Errno::Inval),
		},
	)
}

#[hermit_macro::system(errno)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sys_connect(fd: i32, name: *const sockaddr, namelen: socklen_t) -> i32 {
	if name.is_null() {
		return -i32::from(Errno::Inval);
	}

	let Ok(sa_family) = (unsafe { Af::try_from((*name).sa_family) }) else {
		return -i32::from(Errno::Inval);
	};

	let endpoint = match sa_family {
		#[cfg(feature = "vsock")]
		Af::Vsock => {
			if namelen < u32::try_from(size_of::<sockaddr_vm>()).unwrap() {
				return -i32::from(Errno::Inval);
			}
			Endpoint::Vsock(VsockEndpoint::from(unsafe { *name.cast::<sockaddr_vm>() }))
		}
		_ => {
			return -i32::from(Errno::Inval);
		}
	};

	let obj = get_object(fd);
	obj.map_or_else(
		|e| -i32::from(e),
		|v| {
			block_on(async { v.write().await.connect(endpoint).await }, None)
				.map_or_else(|e| -i32::from(e), |()| 0)
		},
	)
}

#[hermit_macro::system(errno)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sys_getsockname(
	fd: i32,
	addr: *mut sockaddr,
	addrlen: *mut socklen_t,
) -> i32 {
	let obj = get_object(fd);
	obj.map_or_else(
		|e| -i32::from(e),
		|v| {
			if let Ok(Some(endpoint)) = block_on(async { v.read().await.getsockname().await }, None)
			{
				if !addr.is_null() && !addrlen.is_null() {
					let addrlen = unsafe { &mut *addrlen };

					match endpoint {
						#[cfg(feature = "vsock")]
						Endpoint::Vsock(_) => {
							if *addrlen >= u32::try_from(size_of::<sockaddr_vm>()).unwrap() {
								warn!("unsupported device");
								0
							} else {
								-i32::from(Errno::Inval)
							}
						}
					}
				} else {
					-i32::from(Errno::Inval)
				}
			} else {
				-i32::from(Errno::Inval)
			}
		},
	)
}

#[hermit_macro::system(errno)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sys_setsockopt(
	fd: i32,
	level: i32,
	optname: i32,
	optval: *const c_void,
	optlen: socklen_t,
) -> i32 {
	if level == SOL_SOCKET && optname == SO_REUSEADDR {
		return 0;
	}

	let Ok(Ok(level)) = u8::try_from(level).map(Ipproto::try_from) else {
		return -i32::from(Errno::Inval);
	};

	debug!("sys_setsockopt: {fd}, level {level:?}, optname {optname}");

	if level == Ipproto::Tcp
		&& optname == TCP_NODELAY
		&& optlen == u32::try_from(size_of::<i32>()).unwrap()
	{
		if optval.is_null() {
			return -i32::from(Errno::Inval);
		}

		let value = unsafe { *optval.cast::<i32>() };
		let obj = get_object(fd);
		obj.map_or_else(
			|e| -i32::from(e),
			|v| {
				block_on(
					async {
						v.read()
							.await
							.setsockopt(SocketOption::TcpNoDelay, value != 0)
							.await
					},
					None,
				)
				.map_or_else(|e| -i32::from(e), |()| 0)
			},
		)
	} else {
		-i32::from(Errno::Inval)
	}
}

#[hermit_macro::system(errno)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sys_getsockopt(
	fd: i32,
	level: i32,
	optname: i32,
	optval: *mut c_void,
	optlen: *mut socklen_t,
) -> i32 {
	let Ok(Ok(level)) = u8::try_from(level).map(Ipproto::try_from) else {
		return -i32::from(Errno::Inval);
	};

	debug!("sys_getsockopt: {fd}, level {level:?}, optname {optname}");

	if level == Ipproto::Tcp && optname == TCP_NODELAY {
		if optval.is_null() || optlen.is_null() {
			return -i32::from(Errno::Inval);
		}

		let optval = unsafe { &mut *optval.cast::<i32>() };
		let optlen = unsafe { &mut *optlen };
		let obj = get_object(fd);
		obj.map_or_else(
			|e| -i32::from(e),
			|v| {
				block_on(
					async { v.read().await.getsockopt(SocketOption::TcpNoDelay).await },
					None,
				)
				.map_or_else(
					|e| -i32::from(e),
					|value| {
						if value {
							*optval = 1;
						} else {
							*optval = 0;
						}
						*optlen = core::mem::size_of::<i32>().try_into().unwrap();

						0
					},
				)
			},
		)
	} else {
		-i32::from(Errno::Inval)
	}
}

#[hermit_macro::system(errno)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sys_send(s: i32, mem: *const c_void, len: usize, _flags: i32) -> isize {
	unsafe { super::write(s, mem.cast(), len) }
}

fn shutdown(sockfd: i32, how: i32) -> i32 {
	let obj = get_object(sockfd);
	obj.map_or_else(
		|e| -i32::from(e),
		|v| {
			block_on(async { v.read().await.shutdown(how).await }, None)
				.map_or_else(|e| -i32::from(e), |()| 0)
		},
	)
}

#[hermit_macro::system(errno)]
#[unsafe(no_mangle)]
pub extern "C" fn sys_shutdown(sockfd: i32, how: i32) -> i32 {
	shutdown(sockfd, how)
}

#[hermit_macro::system(errno)]
#[unsafe(no_mangle)]
pub extern "C" fn sys_shutdown_socket(fd: i32, how: i32) -> i32 {
	shutdown(fd, how)
}

#[hermit_macro::system(errno)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sys_recv(fd: i32, buf: *mut u8, len: usize, flags: i32) -> isize {
	if flags == 0 {
		let slice = unsafe { core::slice::from_raw_parts_mut(buf.cast(), len) };
		fd::read(fd, slice).map_or_else(
			|e| isize::try_from(-i32::from(e)).unwrap(),
			|v| v.try_into().unwrap(),
		)
	} else {
		(-i32::from(Errno::Inval)).try_into().unwrap()
	}
}
