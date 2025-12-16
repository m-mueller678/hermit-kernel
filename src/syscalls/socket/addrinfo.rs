use alloc::boxed::Box;
use alloc::ffi::CString;
use alloc::vec::Vec;
use core::ffi::{CStr, c_char};
use core::str::FromStr;
use core::{fmt, ptr};

use num_enum::{IntoPrimitive, TryFromPrimitive, TryFromPrimitiveError};

use super::{Af, Ipproto, Sock, SockFlags, sockaddr, sockaddrRef, socklen_t};

#[repr(C)]
#[derive(Default)]
struct addrinfo {
	ai_flags: Ai,
	ai_family: i32,
	ai_socktype: i32,
	ai_protocol: i32,
	ai_addrlen: socklen_t,
	ai_canonname: *mut c_char,
	ai_addr: *mut sockaddr,
	ai_next: Option<Box<addrinfo>>,
}

impl addrinfo {
	fn ai_family(&self) -> Option<Af> {
		let ai_family = u8::try_from(self.ai_family).ok()?;
		Af::try_from(ai_family).ok()
	}

	fn ai_socktype(&self) -> Option<(Sock, SockFlags)> {
		Sock::from_bits(self.ai_socktype)
	}

	fn ai_protocol(&self) -> Option<Ipproto> {
		let ai_protocol = u8::try_from(self.ai_protocol).ok()?;
		Ipproto::try_from(ai_protocol).ok()
	}

	fn ai_addr(&self) -> Option<Result<sockaddrRef<'_>, TryFromPrimitiveError<Af>>> {
		if self.ai_addr.is_null() {
			return None;
		}

		let ai_addr = unsafe { &*ptr::from_ref(&self.ai_addr).cast() };
		let ret = unsafe { sockaddr::as_ref(ai_addr) };
		Some(ret)
	}

	fn ai_canonname(&self) -> Option<&CStr> {
		if self.ai_canonname.is_null() {
			return None;
		}

		let ai_canonname = unsafe { CStr::from_ptr(self.ai_canonname) };
		Some(ai_canonname)
	}
}

impl fmt::Debug for addrinfo {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		f.debug_struct("addrinfo")
			.field("ai_flags", &self.ai_flags)
			.field("ai_family", &self.ai_family())
			.field("ai_socktype", &self.ai_socktype())
			.field("ai_protocol", &self.ai_protocol())
			.field("ai_addrlen", &self.ai_addrlen)
			.field("ai_addr", &self.ai_addr())
			.field("ai_canonname", &self.ai_canonname())
			.finish()
	}
}

impl Drop for addrinfo {
	fn drop(&mut self) {
		if !self.ai_addr.is_null() {
			let ai_addr = unsafe { sockaddr::as_box(self.ai_addr).unwrap() };
			drop(ai_addr);
		}

		if !self.ai_canonname.is_null() {
			let ai_canonname = unsafe { CString::from_raw(self.ai_canonname) };
			drop(ai_canonname);
		}
	}
}

#[derive(Default)]
#[repr(transparent)]
struct addrinfoList(Option<Box<addrinfo>>);

impl addrinfoList {
	fn is_empty(&self) -> bool {
		self.0.is_none()
	}

	fn iter(&self) -> addrinfoIter<'_> {
		addrinfoIter(self.0.as_deref())
	}
}

impl fmt::Debug for addrinfoList {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		f.debug_list().entries(self.iter()).finish()
	}
}

impl Extend<addrinfo> for addrinfoList {
	fn extend<T: IntoIterator<Item = addrinfo>>(&mut self, iter: T) {
		let mut place = &mut self.0;

		while let Some(some) = place {
			place = &mut some.ai_next;
		}

		for addrinfo in iter {
			assert!(addrinfo.ai_next.is_none());

			let addrinfo = place.insert(Box::new(addrinfo));
			place = &mut addrinfo.ai_next;
		}
	}
}

impl FromIterator<addrinfo> for addrinfoList {
	fn from_iter<T: IntoIterator<Item = addrinfo>>(iter: T) -> Self {
		let mut res = Self::default();
		res.extend(iter);
		res
	}
}

struct addrinfoIter<'a>(Option<&'a addrinfo>);

impl<'a> Iterator for addrinfoIter<'a> {
	type Item = &'a addrinfo;

	fn next(&mut self) -> Option<Self::Item> {
		let next = self.0?;
		self.0 = next.ai_next.as_deref();
		Some(next)
	}
}

impl<'a> IntoIterator for &'a addrinfoList {
	type Item = &'a addrinfo;

	type IntoIter = addrinfoIter<'a>;

	fn into_iter(self) -> Self::IntoIter {
		self.iter()
	}
}

bitflags! {
	#[repr(transparent)]
	#[derive(Default, PartialEq, Eq, Clone, Copy, Debug)]
	pub struct Ai: i32 {
		const PASSIVE = 0x001;
		const CANONNAME = 0x002;
		const NUMERICHOST = 0x004;
		const NUMERICSERV = 0x008;
		const ALL = 0x100;
		const ADDRCONFIG = 0x400;
		const V4MAPPED = 0x800;

		// The source may set any bits
		const _ = !0;
	}
}

#[derive(TryFromPrimitive, IntoPrimitive, PartialEq, Eq, Clone, Copy, Debug)]
#[repr(i32)]
enum Eai {
	Again = 2,
	Badflags = 3,
	Fail = 4,
	Family = 5,
	Memory = 6,
	Nodata = 7,
	Noname = 8,
	Service = 9,
	Socktype = 10,
	System = 11,
	Overflow = 14,
}

#[derive(Clone, Copy, Debug)]
struct Service {
	port: u16,
	proto: Ipproto,
	socktype: Sock,
}

fn getaddrinfo_serv(
	name: Option<&str>,
	proto: Ipproto,
	sock: Option<Sock>,
	flags: Ai,
) -> Result<Vec<Service>, Eai> {
	let proto =
		match (sock, proto) {
			(Some(Sock::Stream), Ipproto::Ip | Ipproto::Ipv6 | Ipproto::Tcp)
			| (None, Ipproto::Tcp) => Ipproto::Tcp,
			(Some(Sock::Dgram), Ipproto::Ip | Ipproto::Ipv6 | Ipproto::Udp)
			| (None, Ipproto::Udp) => Ipproto::Udp,
			(Some(_), _) => return Err(Eai::Service),
			(None, proto @ (Ipproto::Ip | Ipproto::Ipv6)) => proto,
		};

	let Some(servname) = name else {
		let mut services = vec![];

		if matches!(proto, Ipproto::Ip | Ipproto::Ipv6 | Ipproto::Tcp) {
			services.push(Service {
				port: 0,
				proto: Ipproto::Tcp,
				socktype: Sock::Stream,
			});
		}

		if matches!(proto, Ipproto::Ip | Ipproto::Ipv6 | Ipproto::Udp) {
			services.push(Service {
				port: 0,
				proto: Ipproto::Udp,
				socktype: Sock::Dgram,
			});
		}

		return Ok(services);
	};

	if let Ok(port) = u16::from_str(servname) {
		let mut services = vec![];

		if matches!(proto, Ipproto::Ip | Ipproto::Ipv6 | Ipproto::Tcp) {
			services.push(Service {
				port,
				proto: Ipproto::Tcp,
				socktype: Sock::Stream,
			});
		}

		if matches!(proto, Ipproto::Ip | Ipproto::Ipv6 | Ipproto::Udp) {
			services.push(Service {
				port,
				proto: Ipproto::Udp,
				socktype: Sock::Dgram,
			});
		}

		return Ok(services);
	}

	if flags.contains(Ai::NUMERICSERV) {
		return Err(Eai::Noname);
	}

	// TODO(mkroening): add proper database
	let servname_options = match servname {
		"http" => [Service {
			port: 80,
			proto: Ipproto::Tcp,
			socktype: Sock::Stream,
		}]
		.as_slice(),
		"https" => [
			Service {
				port: 443,
				proto: Ipproto::Tcp,
				socktype: Sock::Stream,
			},
			Service {
				port: 443,
				proto: Ipproto::Udp,
				socktype: Sock::Dgram,
			},
		]
		.as_slice(),
		servname => {
			error!("Unknown service name: {servname}");
			return Err(Eai::Service);
		}
	};

	let services = servname_options
		.iter()
		.copied()
		.filter(|service| {
			matches!(
				(service.proto, proto),
				(Ipproto::Tcp, Ipproto::Ip | Ipproto::Ipv6 | Ipproto::Tcp)
					| (Ipproto::Udp, Ipproto::Ip | Ipproto::Ipv6 | Ipproto::Udp)
			)
		})
		.collect::<Vec<_>>();

	if services.is_empty() {
		return Err(Eai::Service);
	}

	Ok(services)
}
