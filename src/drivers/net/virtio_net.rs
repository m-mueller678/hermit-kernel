//! A module containing a virtio network driver.
//!
//! The module contains ...

use alloc::boxed::Box;
use alloc::rc::Rc;
use alloc::vec::Vec;
use core::cmp::Ordering;
use core::mem;

use align_address::Align;
use pci_types::InterruptLine;
use smoltcp::phy::{Checksum, ChecksumCapabilities};
use smoltcp::wire::{EthernetFrame, Ipv4Packet, Ipv6Packet, ETHERNET_HEADER_LEN};
use virtio_def::features::VirtioF;
use zerocopy::AsBytes;

use self::constants::{FeatureSet, Features, NetHdrFlag, NetHdrGSO, Status, MAX_NUM_VQ};
use self::error::VirtioNetError;
#[cfg(not(target_arch = "riscv64"))]
use crate::arch::kernel::core_local::increment_irq_counter;
use crate::config::VIRTIO_MAX_QUEUE_SIZE;
#[cfg(not(feature = "pci"))]
use crate::drivers::net::virtio_mmio::NetDevCfgRaw;
#[cfg(feature = "pci")]
use crate::drivers::net::virtio_pci::NetDevCfgRaw;
use crate::drivers::net::NetworkDriver;
#[cfg(not(feature = "pci"))]
use crate::drivers::virtio::transport::mmio::{ComCfg, IsrStatus, NotifCfg};
#[cfg(feature = "pci")]
use crate::drivers::virtio::transport::pci::{ComCfg, IsrStatus, NotifCfg};
use crate::drivers::virtio::virtqueue::packed::PackedVq;
use crate::drivers::virtio::virtqueue::split::SplitVq;
use crate::drivers::virtio::virtqueue::{BuffSpec, BufferToken, Bytes, Virtq, VqIndex, VqSize};
use crate::executor::device::{RxToken, TxToken};

/// A wrapper struct for the raw configuration structure.
/// Handling the right access to fields, as some are read-only
/// for the driver.
pub(crate) struct NetDevCfg {
	pub raw: &'static NetDevCfgRaw,
	pub dev_id: u16,

	// Feature booleans
	pub features: FeatureSet,
}

#[derive(AsBytes, Debug)]
#[repr(C)]
pub struct VirtioNetHdr {
	flags: NetHdrFlag,
	gso_type: NetHdrGSO,
	/// Ethernet + IP + tcp/udp hdrs
	hdr_len: u16,
	/// Bytes to append to hdr_len per frame
	gso_size: u16,
	/// Position to start checksumming from
	csum_start: u16,
	/// Offset after that to place checksum
	csum_offset: u16,
	/// Number of buffers this Packet consists of
	num_buffers: u16,
}

impl Default for VirtioNetHdr {
	fn default() -> Self {
		Self {
			flags: NetHdrFlag::VIRTIO_NET_HDR_F_NONE,
			gso_type: NetHdrGSO::VIRTIO_NET_HDR_GSO_NONE,
			hdr_len: 0,
			gso_size: 0,
			csum_start: 0,
			csum_offset: 0,
			num_buffers: 0,
		}
	}
}

pub struct CtrlQueue(Option<Rc<dyn Virtq>>);

impl CtrlQueue {
	pub fn new(vq: Option<Rc<dyn Virtq>>) -> Self {
		CtrlQueue(vq)
	}
}

#[allow(dead_code, non_camel_case_types)]
#[derive(Copy, Clone, Debug)]
#[repr(u8)]
enum CtrlClass {
	VIRTIO_NET_CTRL_RX = 1 << 0,
	VIRTIO_NET_CTRL_MAC = 1 << 1,
	VIRTIO_NET_CTRL_VLAN = 1 << 2,
	VIRTIO_NET_CTRL_ANNOUNCE = 1 << 3,
	VIRTIO_NET_CTRL_MQ = 1 << 4,
}

impl From<CtrlClass> for u8 {
	fn from(val: CtrlClass) -> Self {
		match val {
			CtrlClass::VIRTIO_NET_CTRL_RX => 1 << 0,
			CtrlClass::VIRTIO_NET_CTRL_MAC => 1 << 1,
			CtrlClass::VIRTIO_NET_CTRL_VLAN => 1 << 2,
			CtrlClass::VIRTIO_NET_CTRL_ANNOUNCE => 1 << 3,
			CtrlClass::VIRTIO_NET_CTRL_MQ => 1 << 4,
		}
	}
}

#[allow(dead_code, non_camel_case_types)]
#[derive(Copy, Clone, Debug)]
#[repr(u8)]
enum RxCmd {
	VIRTIO_NET_CTRL_RX_PROMISC = 1 << 0,
	VIRTIO_NET_CTRL_RX_ALLMULTI = 1 << 1,
	VIRTIO_NET_CTRL_RX_ALLUNI = 1 << 2,
	VIRTIO_NET_CTRL_RX_NOMULTI = 1 << 3,
	VIRTIO_NET_CTRL_RX_NOUNI = 1 << 4,
	VIRTIO_NET_CTRL_RX_NOBCAST = 1 << 5,
}

#[allow(dead_code, non_camel_case_types)]
#[derive(Copy, Clone, Debug)]
#[repr(u8)]
enum MacCmd {
	VIRTIO_NET_CTRL_MAC_TABLE_SET = 1 << 0,
	VIRTIO_NET_CTRL_MAC_ADDR_SET = 1 << 1,
}

#[allow(dead_code, non_camel_case_types)]
#[derive(Copy, Clone, Debug)]
#[repr(u8)]
enum VlanCmd {
	VIRTIO_NET_CTRL_VLAN_ADD = 1 << 0,
	VIRTIO_NET_CTRL_VLAN_DEL = 1 << 1,
}

#[allow(dead_code, non_camel_case_types)]
#[derive(Copy, Clone, Debug)]
#[repr(u8)]
enum AnceCmd {
	VIRTIO_NET_CTRL_ANNOUNCE_ACK = 1 << 0,
}

#[allow(dead_code, non_camel_case_types)]
#[derive(Copy, Clone, Debug)]
#[repr(u8)]
enum MqCmd {
	VIRTIO_NET_CTRL_MQ_VQ_PAIRS_SET = 1 << 0,
	VIRTIO_NET_CTRL_MQ_VQ_PAIRS_MIN = 1 << 1,
	VIRTIO_NET_CTRL_MQ_VQ_PAIRS_MAX = 0x80,
}

pub struct RxQueues {
	vqs: Vec<Rc<dyn Virtq>>,
	poll_sender: async_channel::Sender<Box<BufferToken>>,
	poll_receiver: async_channel::Receiver<Box<BufferToken>>,
	is_multi: bool,
}

impl RxQueues {
	pub fn new(vqs: Vec<Rc<dyn Virtq>>, is_multi: bool) -> Self {
		let (poll_sender, poll_receiver) = async_channel::unbounded();
		Self {
			vqs,
			poll_sender,
			poll_receiver,
			is_multi,
		}
	}

	/// Takes care if handling packets correctly which need some processing after being received.
	/// This currently include nothing. But in the future it might include among others::
	/// * Calculating missing checksums
	/// * Merging receive buffers, by simply checking the poll_queue (if VIRTIO_NET_F_MRG_BUF)
	fn post_processing(buffer_tkn: Box<BufferToken>) -> Result<Box<BufferToken>, VirtioNetError> {
		Ok(buffer_tkn)
	}

	/// Adds a given queue to the underlying vector and populates the queue with RecvBuffers.
	///
	/// Queues are all populated according to Virtio specification v1.1. - 5.1.6.3.1
	fn add(&mut self, vq: Rc<dyn Virtq>, dev_cfg: &NetDevCfg) {
		let num_buff: u16 = vq.size().into();

		let rx_size = if dev_cfg
			.features
			.is_feature(Features::VIRTIO_NET_F_MRG_RXBUF)
		{
			(1514 + mem::size_of::<VirtioNetHdr>())
				.align_up(core::mem::size_of::<crossbeam_utils::CachePadded<u8>>())
		} else {
			dev_cfg.raw.get_mtu() as usize + mem::size_of::<VirtioNetHdr>()
		};

		// See Virtio specification v1.1 - 5.1.6.3.1
		//
		let spec = BuffSpec::Single(Bytes::new(rx_size).unwrap());
		for _ in 0..num_buff {
			let buff_tkn = match vq.clone().prep_buffer(None, Some(spec.clone())) {
				Ok(tkn) => tkn,
				Err(_vq_err) => {
					error!("Setup of network queue failed, which should not happen!");
					panic!("setup of network queue failed!");
				}
			};

			// BufferTokens are directly provided to the queue
			// TransferTokens are directly dispatched
			// Transfers will be awaited at the queue
			buff_tkn
				.provide()
				.dispatch_await(self.poll_sender.clone(), false);
		}

		// Safe virtqueue
		self.vqs.push(vq);

		if self.vqs.len() > 1 {
			self.is_multi = true;
		}
	}

	fn get_next(&mut self) -> Option<Box<BufferToken>> {
		let transfer = self.poll_receiver.try_recv();

		transfer
			.or_else(|_| {
				// Check if any not yet provided transfers are in the queue.
				self.poll();

				self.poll_receiver.try_recv()
			})
			.ok()
	}

	fn poll(&self) {
		if self.is_multi {
			for vq in &self.vqs {
				vq.poll();
			}
		} else {
			self.vqs[0].poll();
		}
	}

	fn enable_notifs(&self) {
		if self.is_multi {
			for vq in &self.vqs {
				vq.enable_notifs();
			}
		} else {
			self.vqs[0].enable_notifs();
		}
	}

	fn disable_notifs(&self) {
		if self.is_multi {
			for vq in &self.vqs {
				vq.disable_notifs();
			}
		} else {
			self.vqs[0].disable_notifs();
		}
	}
}

/// Structure which handles transmission of packets and delegation
/// to the respective queue structures.
pub struct TxQueues {
	vqs: Vec<Rc<dyn Virtq>>,
	poll_sender: async_channel::Sender<Box<BufferToken>>,
	poll_receiver: async_channel::Receiver<Box<BufferToken>>,
	ready_queue: Vec<BufferToken>,
	/// Indicates, whether the Driver/Device are using multiple
	/// queues for communication.
	is_multi: bool,
}

impl TxQueues {
	pub fn new(vqs: Vec<Rc<dyn Virtq>>, ready_queue: Vec<BufferToken>, is_multi: bool) -> Self {
		let (poll_sender, poll_receiver) = async_channel::unbounded();
		Self {
			vqs,
			poll_sender,
			poll_receiver,
			ready_queue,
			is_multi,
		}
	}
	#[allow(dead_code)]
	fn enable_notifs(&self) {
		if self.is_multi {
			for vq in &self.vqs {
				vq.enable_notifs();
			}
		} else {
			self.vqs[0].enable_notifs();
		}
	}

	#[allow(dead_code)]
	fn disable_notifs(&self) {
		if self.is_multi {
			for vq in &self.vqs {
				vq.disable_notifs();
			}
		} else {
			self.vqs[0].disable_notifs();
		}
	}

	fn poll(&self) {
		if self.is_multi {
			for vq in &self.vqs {
				vq.poll();
			}
		} else {
			self.vqs[0].poll();
		}
	}

	fn add(&mut self, vq: Rc<dyn Virtq>, dev_cfg: &NetDevCfg) {
		// Safe virtqueue
		self.vqs.push(vq.clone());
		if self.vqs.len() == 1 {
			// Unwrapping is safe, as one virtq will be definitely in the vector.
			let vq = self.vqs.first().unwrap();

			if dev_cfg
				.features
				.is_feature(Features::VIRTIO_NET_F_GUEST_TSO4)
				| dev_cfg
					.features
					.is_feature(Features::VIRTIO_NET_F_GUEST_TSO6)
				| dev_cfg
					.features
					.is_feature(Features::VIRTIO_NET_F_GUEST_UFO)
			{
				// Virtio specification v1.1. - 5.1.6.2 point 5.
				//      Header and data are added as ONE output descriptor to the transmitvq.
				//      Hence we are interpreting this, as the fact, that send packets must be inside a single descriptor.
				// As usize is currently safe as the minimal usize is defined as 16bit in rust.
				let buff_def = Bytes::new(mem::size_of::<VirtioNetHdr>() + 65550).unwrap();
				let spec = BuffSpec::Single(buff_def);

				let num_buff: u16 = vq.size().into();

				for _ in 0..num_buff {
					self.ready_queue.push(
						vq.clone()
							.prep_buffer(Some(spec.clone()), None)
							.unwrap()
							.write_seq(Some(&VirtioNetHdr::default()), None::<&VirtioNetHdr>)
							.unwrap(),
					)
				}
			} else {
				// Virtio specification v1.1. - 5.1.6.2 point 5.
				//      Header and data are added as ONE output descriptor to the transmitvq.
				//      Hence we are interpreting this, as the fact, that send packets must be inside a single descriptor.
				// As usize is currently safe as the minimal usize is defined as 16bit in rust.
				let buff_def =
					Bytes::new(mem::size_of::<VirtioNetHdr>() + dev_cfg.raw.get_mtu() as usize)
						.unwrap();
				let spec = BuffSpec::Single(buff_def);

				let num_buff: u16 = vq.size().into();

				for _ in 0..num_buff {
					self.ready_queue.push(
						vq.clone()
							.prep_buffer(Some(spec.clone()), None)
							.unwrap()
							.write_seq(Some(&VirtioNetHdr::default()), None::<&VirtioNetHdr>)
							.unwrap(),
					)
				}
			}
		} else {
			self.is_multi = true;
			// Currently we are doing nothing with the additional queues. They are inactive and might be used in the
			// future
		}
	}

	/// Returns either a buffertoken and the corresponding index of the
	/// virtqueue it is coming from. (Index in the TxQueues.vqs vector)
	///
	/// OR returns None, if no Buffertoken could be generated
	fn get_tkn(&mut self, len: usize) -> Option<(BufferToken, usize)> {
		// Check all ready token, for correct size.
		// Drop token if not so
		//
		// All Tokens inside the ready_queue are coming from the main queue with index 0.
		while let Some(mut tkn) = self.ready_queue.pop() {
			let (send_len, _) = tkn.len();

			match send_len.cmp(&len) {
				Ordering::Less => {}
				Ordering::Equal => return Some((tkn, 0)),
				Ordering::Greater => {
					tkn.restr_size(Some(len), None).unwrap();
					return Some((tkn, 0));
				}
			}
		}

		if self.poll_receiver.is_empty() {
			self.poll();
		}

		while let Ok(buffer_token) = self.poll_receiver.try_recv() {
			let mut tkn = buffer_token.reset();
			let (send_len, _) = tkn.len();

			match send_len.cmp(&len) {
				Ordering::Less => {}
				Ordering::Equal => return Some((tkn, 0)),
				Ordering::Greater => {
					tkn.restr_size(Some(len), None).unwrap();
					return Some((tkn, 0));
				}
			}
		}

		// As usize is currently safe as the minimal usize is defined as 16bit in rust.
		let spec = BuffSpec::Single(Bytes::new(len).unwrap());

		match self.vqs[0].clone().prep_buffer(Some(spec), None) {
			Ok(tkn) => Some((tkn, 0)),
			Err(_) => {
				// Here it is possible if multiple queues are enabled to get another buffertoken from them!
				// Info the queues are disabled upon initialization and should be enabled somehow!
				None
			}
		}
	}
}

/// Virtio network driver struct.
///
/// Struct allows to control devices virtqueues as also
/// the device itself.
pub(crate) struct VirtioNetDriver {
	pub(super) dev_cfg: NetDevCfg,
	pub(super) com_cfg: ComCfg,
	pub(super) isr_stat: IsrStatus,
	pub(super) notif_cfg: NotifCfg,

	pub(super) ctrl_vq: CtrlQueue,
	pub(super) recv_vqs: RxQueues,
	pub(super) send_vqs: TxQueues,

	pub(super) num_vqs: u16,
	#[cfg_attr(target_arch = "riscv64", allow(dead_code))]
	pub(super) irq: InterruptLine,
	pub(super) mtu: u16,
	pub(super) checksums: ChecksumCapabilities,
}

impl NetworkDriver for VirtioNetDriver {
	/// Returns the mac address of the device.
	/// If VIRTIO_NET_F_MAC is not set, the function panics currently!
	fn get_mac_address(&self) -> [u8; 6] {
		if self.dev_cfg.features.is_feature(Features::VIRTIO_NET_F_MAC) {
			self.dev_cfg.raw.get_mac()
		} else {
			unreachable!("Currently VIRTIO_NET_F_MAC must be negotiated!")
		}
	}

	/// Returns the current MTU of the device.
	fn get_mtu(&self) -> u16 {
		self.mtu
	}

	fn get_checksums(&self) -> ChecksumCapabilities {
		self.checksums.clone()
	}

	#[allow(dead_code)]
	fn has_packet(&self) -> bool {
		self.recv_vqs.poll();
		!self.recv_vqs.poll_receiver.is_empty()
	}

	/// Provides smoltcp a slice to copy the IP packet and transfer the packet
	/// to the send queue.
	fn send_packet<R, F>(&mut self, len: usize, f: F) -> R
	where
		F: FnOnce(&mut [u8]) -> R,
	{
		if let Some((mut buff_tkn, _vq_index)) = self
			.send_vqs
			.get_tkn(len + core::mem::size_of::<VirtioNetHdr>())
		{
			let (send_ptrs, _) = buff_tkn.raw_ptrs();
			// Currently we have single Buffers in the TxQueue of size: MTU + ETHERNET_HEADER_LEN + VIRTIO_NET_HDR
			// see TxQueue.add()
			let (buff_ptr, _) = send_ptrs.unwrap()[0];

			// Do not show smoltcp the memory region for VirtioNetHdr.
			let header = unsafe { &mut *(buff_ptr as *mut VirtioNetHdr) };
			*header = Default::default();
			let buff_ptr = unsafe {
				buff_ptr.offset(isize::try_from(core::mem::size_of::<VirtioNetHdr>()).unwrap())
			};

			let buf_slice: &'static mut [u8] =
				unsafe { core::slice::from_raw_parts_mut(buff_ptr, len) };
			let result = f(buf_slice);

			// If a checksum isn't necessary, we have inform the host within the header
			// see Virtio specification 5.1.6.2
			if !self.checksums.tcp.tx() || !self.checksums.udp.tx() {
				header.flags = NetHdrFlag::VIRTIO_NET_HDR_F_NEEDS_CSUM;
				let ethernet_frame: smoltcp::wire::EthernetFrame<&[u8]> =
					EthernetFrame::new_unchecked(buf_slice);
				let packet_header_len: u16;
				let protocol;
				match ethernet_frame.ethertype() {
					smoltcp::wire::EthernetProtocol::Ipv4 => {
						let packet = Ipv4Packet::new_unchecked(ethernet_frame.payload());
						packet_header_len = packet.header_len().into();
						protocol = Some(packet.next_header());
					}
					smoltcp::wire::EthernetProtocol::Ipv6 => {
						let packet = Ipv6Packet::new_unchecked(ethernet_frame.payload());
						packet_header_len = packet.header_len().try_into().unwrap();
						protocol = Some(packet.next_header());
					}
					_ => {
						packet_header_len = 0;
						protocol = None;
					}
				}
				header.csum_start = u16::try_from(ETHERNET_HEADER_LEN).unwrap() + packet_header_len;
				header.csum_offset = match protocol {
					Some(smoltcp::wire::IpProtocol::Tcp) => 16,
					Some(smoltcp::wire::IpProtocol::Udp) => 6,
					_ => 0,
				};
			}

			buff_tkn
				.provide()
				.dispatch_await(self.send_vqs.poll_sender.clone(), false);

			result
		} else {
			panic!("Unable to get token for send queue");
		}
	}

	fn receive_packet(&mut self) -> Option<(RxToken, TxToken)> {
		match self.recv_vqs.get_next() {
			Some(transfer) => {
				let transfer = match RxQueues::post_processing(transfer) {
					Ok(trf) => trf,
					Err(vnet_err) => {
						warn!("Post processing failed. Err: {:?}", vnet_err);
						return None;
					}
				};

				let (_, recv_data_opt) = transfer.as_slices().unwrap();
				let mut recv_data = recv_data_opt.unwrap();

				// If the given length isn't 1, we currently fail.
				if recv_data.len() == 1 {
					let mut vec_data: Vec<u8> = Vec::with_capacity(self.mtu.into());
					let num_buffers = {
						const HEADER_SIZE: usize = mem::size_of::<VirtioNetHdr>();
						let packet = recv_data.pop().unwrap();

						// drop packets with invalid packet size
						if packet.len() < HEADER_SIZE {
							transfer
								.reset()
								.provide()
								.dispatch_await(self.recv_vqs.poll_sender.clone(), false);

							return None;
						}

						let header = unsafe {
							core::mem::transmute::<[u8; HEADER_SIZE], VirtioNetHdr>(
								packet[..HEADER_SIZE].try_into().unwrap(),
							)
						};
						trace!("Header: {:?}", header);
						let num_buffers = header.num_buffers;

						vec_data.extend_from_slice(&packet[mem::size_of::<VirtioNetHdr>()..]);
						transfer
							.reset()
							.provide()
							.dispatch_await(self.recv_vqs.poll_sender.clone(), false);

						num_buffers
					};

					for _ in 1..num_buffers {
						let transfer =
							match RxQueues::post_processing(self.recv_vqs.get_next().unwrap()) {
								Ok(trf) => trf,
								Err(vnet_err) => {
									warn!("Post processing failed. Err: {:?}", vnet_err);
									return None;
								}
							};

						let (_, recv_data_opt) = transfer.as_slices().unwrap();
						let mut recv_data = recv_data_opt.unwrap();
						let packet = recv_data.pop().unwrap();
						vec_data.extend_from_slice(packet);
						transfer
							.reset()
							.provide()
							.dispatch_await(self.recv_vqs.poll_sender.clone(), false);
					}

					Some((RxToken::new(vec_data), TxToken::new()))
				} else {
					error!("Empty transfer, or with wrong buffer layout. Reusing and returning error to user-space network driver...");
					transfer
						.reset()
						.write_seq(None::<&VirtioNetHdr>, Some(&VirtioNetHdr::default()))
						.unwrap()
						.provide()
						.dispatch_await(self.recv_vqs.poll_sender.clone(), false);

					None
				}
			}
			None => None,
		}
	}

	fn set_polling_mode(&mut self, value: bool) {
		if value {
			self.disable_interrupts();
		} else {
			self.enable_interrupts();
		}
	}

	fn handle_interrupt(&mut self) -> bool {
		#[cfg(not(target_arch = "riscv64"))]
		increment_irq_counter(32 + self.irq);

		let result = if self.isr_stat.is_interrupt() {
			true
		} else if self.isr_stat.is_cfg_change() {
			info!("Configuration changes are not possible! Aborting");
			todo!("Implement possibility to change config on the fly...")
		} else {
			false
		};

		self.isr_stat.acknowledge();

		result
	}
}

// Backend-independent interface for Virtio network driver
impl VirtioNetDriver {
	#[cfg(feature = "pci")]
	pub fn get_dev_id(&self) -> u16 {
		self.dev_cfg.dev_id
	}

	#[cfg(feature = "pci")]
	pub fn set_failed(&mut self) {
		self.com_cfg.set_failed();
	}

	/// Returns the current status of the device, if VIRTIO_NET_F_STATUS
	/// has been negotiated. Otherwise assumes an active device.
	#[cfg(not(feature = "pci"))]
	pub fn dev_status(&self) -> u16 {
		if self
			.dev_cfg
			.features
			.is_feature(Features::VIRTIO_NET_F_STATUS)
		{
			self.dev_cfg.raw.get_status()
		} else {
			u16::from(Status::VIRTIO_NET_S_LINK_UP)
		}
	}

	/// Returns the links status.
	/// If feature VIRTIO_NET_F_STATUS has not been negotiated, then we assume the link is up!
	#[cfg(feature = "pci")]
	pub fn is_link_up(&self) -> bool {
		if self
			.dev_cfg
			.features
			.is_feature(Features::VIRTIO_NET_F_STATUS)
		{
			self.dev_cfg.raw.get_status() & u16::from(Status::VIRTIO_NET_S_LINK_UP)
				== u16::from(Status::VIRTIO_NET_S_LINK_UP)
		} else {
			true
		}
	}

	#[allow(dead_code)]
	pub fn is_announce(&self) -> bool {
		if self
			.dev_cfg
			.features
			.is_feature(Features::VIRTIO_NET_F_STATUS)
		{
			self.dev_cfg.raw.get_status() & u16::from(Status::VIRTIO_NET_S_ANNOUNCE)
				== u16::from(Status::VIRTIO_NET_S_ANNOUNCE)
		} else {
			false
		}
	}

	/// Returns the maximal number of virtqueue pairs allowed. This is the
	/// dominant setting to define the number of virtqueues for the network
	/// device and overrides the num_vq field in the common config.
	///
	/// Returns 1 (i.e. minimum number of pairs) if VIRTIO_NET_F_MQ is not set.
	#[allow(dead_code)]
	pub fn get_max_vq_pairs(&self) -> u16 {
		if self.dev_cfg.features.is_feature(Features::VIRTIO_NET_F_MQ) {
			self.dev_cfg.raw.get_max_virtqueue_pairs()
		} else {
			1
		}
	}

	pub fn disable_interrupts(&self) {
		// For send and receive queues?
		// Only for receive? Because send is off anyway?
		self.recv_vqs.disable_notifs();
	}

	pub fn enable_interrupts(&self) {
		// For send and receive queues?
		// Only for receive? Because send is off anyway?
		self.recv_vqs.enable_notifs();
	}

	/// Initializes the device in adherence to specification. Returns Some(VirtioNetError)
	/// upon failure and None in case everything worked as expected.
	///
	/// See Virtio specification v1.1. - 3.1.1.
	///                      and v1.1. - 5.1.5
	pub fn init_dev(&mut self) -> Result<(), VirtioNetError> {
		// Reset
		self.com_cfg.reset_dev();

		// Indicate device, that OS noticed it
		self.com_cfg.ack_dev();

		// Indicate device, that driver is able to handle it
		self.com_cfg.set_drv();

		// Define minimal feature set
		let minimal_features: Vec<Features> =
			vec![Features::VIRTIO_F_VERSION_1, Features::VIRTIO_NET_F_MAC];

		let mut minimal_feature_set = FeatureSet::new(0);
		minimal_feature_set.set_features(&minimal_features);
		let mut features: Vec<Features> = minimal_features;

		// If wanted, push new features into features here:
		//
		// the link status can be announced
		features.push(Features::VIRTIO_NET_F_STATUS);
		// Indirect descriptors can be used
		features.push(Features::VIRTIO_F_RING_INDIRECT_DESC);
		// MTU setting can be used
		features.push(Features::VIRTIO_NET_F_MTU);
		// Packed Vq can be used
		features.push(Features::VIRTIO_F_RING_PACKED);
		// Guest avoids the creation of checksums
		features.push(Features::VIRTIO_NET_F_GUEST_CSUM);
		// Host should avoid the creation of checksums
		features.push(Features::VIRTIO_NET_F_CSUM);
		// Driver can merge receive buffers
		features.push(Features::VIRTIO_NET_F_MRG_RXBUF);

		// Currently the driver does NOT support the features below.
		// In order to provide functionality for these, the driver
		// needs to take care of calculating checksum in
		// RxQueues.post_processing()
		// features.push(Features::VIRTIO_NET_F_GUEST_TSO4);
		// features.push(Features::VIRTIO_NET_F_GUEST_TSO6);

		// Negotiate features with device. Automatically reduces selected features in order to meet device capabilities.
		// Aborts in case incompatible features are selected by the driver or the device does not support minimal_feature_set.
		match self.negotiate_features(&features) {
			Ok(_) => info!(
				"Driver found a subset of features for virtio device {:x}. Features are: {:?}",
				self.dev_cfg.dev_id, &features
			),
			Err(vnet_err) => {
				match vnet_err {
					VirtioNetError::FeatureRequirementsNotMet(feature_set) => {
						error!("Network drivers feature set {:x} does not satisfy rules in section 5.1.3.1 of specification v1.1. Aborting!", u64::from(feature_set));
						return Err(vnet_err);
					}
					VirtioNetError::IncompatibleFeatureSets(driver_features, device_features) => {
						// Create a new matching feature set for device and driver if the minimal set is met!
						if (minimal_feature_set & device_features) != minimal_feature_set {
							error!("Device features set, does not satisfy minimal features needed. Aborting!");
							return Err(VirtioNetError::FailFeatureNeg(self.dev_cfg.dev_id));
						} else {
							features = match Features::from_set(device_features & driver_features) {
								Some(features) => features,
								None => {
									error!("Feature negotiation failed with minimal feature set. Aborting!");
									return Err(VirtioNetError::FailFeatureNeg(
										self.dev_cfg.dev_id,
									));
								}
							};

							match self.negotiate_features(&features) {
                                Ok(_) => info!("Driver found a subset of features for virtio device {:x}. Features are: {:?}", self.dev_cfg.dev_id, &features),
                                Err(vnet_err) => {
                                    match vnet_err {
                                        VirtioNetError::FeatureRequirementsNotMet(feature_set) => {
                                            error!("Network device offers a feature set {:x} when used completely does not satisfy rules in section 5.1.3.1 of specification v1.1. Aborting!", u64::from(feature_set));
                                            return Err(vnet_err);
                                        },
                                        _ => {
                                            error!("Feature Set after reduction still not usable. Set: {:?}. Aborting!", features);
                                            return Err(vnet_err);
                                        }
                                    }
                                }
                            }
						}
					}
					_ => {
						error!(
							"Wanted set of features is NOT supported by device. Set: {:?}",
							features
						);
						return Err(vnet_err);
					}
				}
			}
		}

		// Indicates the device, that the current feature set is final for the driver
		// and will not be changed.
		self.com_cfg.features_ok();

		// Checks if the device has accepted final set. This finishes feature negotiation.
		if self.com_cfg.check_features() {
			info!(
				"Features have been negotiated between virtio network device {:x} and driver.",
				self.dev_cfg.dev_id
			);
			// Set feature set in device config fur future use.
			self.dev_cfg.features.set_features(&features);
		} else {
			return Err(VirtioNetError::FailFeatureNeg(self.dev_cfg.dev_id));
		}

		match self.dev_spec_init() {
			Ok(_) => info!(
				"Device specific initialization for Virtio network device {:x} finished",
				self.dev_cfg.dev_id
			),
			Err(vnet_err) => return Err(vnet_err),
		}
		// At this point the device is "live"
		self.com_cfg.drv_ok();

		if self
			.dev_cfg
			.features
			.is_feature(Features::VIRTIO_NET_F_CSUM)
			&& self
				.dev_cfg
				.features
				.is_feature(Features::VIRTIO_NET_F_GUEST_CSUM)
		{
			self.checksums.udp = Checksum::None;
			self.checksums.tcp = Checksum::None;
		} else if self
			.dev_cfg
			.features
			.is_feature(Features::VIRTIO_NET_F_CSUM)
		{
			self.checksums.udp = Checksum::Rx;
			self.checksums.tcp = Checksum::Rx;
		} else if self
			.dev_cfg
			.features
			.is_feature(Features::VIRTIO_NET_F_GUEST_CSUM)
		{
			self.checksums.udp = Checksum::Tx;
			self.checksums.tcp = Checksum::Tx;
		}
		debug!("{:?}", self.checksums);

		if self.dev_cfg.features.is_feature(Features::VIRTIO_NET_F_MTU) {
			self.mtu = self.dev_cfg.raw.get_mtu();
		}

		Ok(())
	}

	/// Negotiates a subset of features, understood and wanted by both the OS
	/// and the device.
	fn negotiate_features(&mut self, wanted_features: &[Features]) -> Result<(), VirtioNetError> {
		let mut driver_features = FeatureSet::new(0);

		for feature in wanted_features.iter() {
			driver_features |= *feature;
		}

		let device_features = FeatureSet::new(self.com_cfg.dev_features());

		// Checks if the selected feature set is compatible with requirements for
		// features according to Virtio spec. v1.1 - 5.1.3.1.
		match FeatureSet::check_features(wanted_features) {
			Ok(_) => {
				info!("Feature set wanted by network driver are in conformance with specification.")
			}
			Err(vnet_err) => return Err(vnet_err),
		}

		if (device_features & driver_features) == driver_features {
			// If device supports subset of features write feature set to common config
			self.com_cfg.set_drv_features(driver_features.into());
			Ok(())
		} else {
			Err(VirtioNetError::IncompatibleFeatureSets(
				driver_features,
				device_features,
			))
		}
	}

	/// Device Specific initialization according to Virtio specifictation v1.1. - 5.1.5
	fn dev_spec_init(&mut self) -> Result<(), VirtioNetError> {
		match self.virtqueue_init() {
			Ok(_) => info!("Network driver successfully initialized virtqueues."),
			Err(vnet_err) => return Err(vnet_err),
		}

		// Add a control if feature is negotiated
		if self
			.dev_cfg
			.features
			.is_feature(Features::VIRTIO_NET_F_CTRL_VQ)
		{
			if self
				.dev_cfg
				.features
				.is_feature(Features::VIRTIO_F_RING_PACKED)
			{
				self.ctrl_vq = CtrlQueue(Some(Rc::new(
					PackedVq::new(
						&mut self.com_cfg,
						&self.notif_cfg,
						VqSize::from(VIRTIO_MAX_QUEUE_SIZE),
						VqIndex::from(self.num_vqs),
						VirtioF::from_bits_retain(u64::from(self.dev_cfg.features).into()),
					)
					.unwrap(),
				)));
			} else {
				self.ctrl_vq = CtrlQueue(Some(Rc::new(
					SplitVq::new(
						&mut self.com_cfg,
						&self.notif_cfg,
						VqSize::from(VIRTIO_MAX_QUEUE_SIZE),
						VqIndex::from(self.num_vqs),
						VirtioF::from_bits_retain(u64::from(self.dev_cfg.features).into()),
					)
					.unwrap(),
				)));
			}

			self.ctrl_vq.0.as_ref().unwrap().enable_notifs();
		}

		Ok(())
	}

	/// Initialize virtqueues via the queue interface and populates receiving queues
	fn virtqueue_init(&mut self) -> Result<(), VirtioNetError> {
		// We are assuming here, that the device single source of truth is the
		// device specific configuration. Hence we do NOT check if
		//
		// max_virtqueue_pairs + 1 < num_queues
		//
		// - the plus 1 is due to the possibility of an existing control queue
		// - the num_queues is found in the ComCfg struct of the device and defines the maximal number
		// of supported queues.
		if self.dev_cfg.features.is_feature(Features::VIRTIO_NET_F_MQ) {
			if self.dev_cfg.raw.get_max_virtqueue_pairs() * 2 >= MAX_NUM_VQ {
				self.num_vqs = MAX_NUM_VQ;
			} else {
				self.num_vqs = self.dev_cfg.raw.get_max_virtqueue_pairs() * 2;
			}
		} else {
			// Minimal number of virtqueues defined in the standard v1.1. - 5.1.5 Step 1
			self.num_vqs = 2;
		}

		// The loop is running from 0 to num_vqs and the indexes are provided to the VqIndex::from function in this way
		// in order to allow the indexes of the queues to be in a form of:
		//
		// index i for receiv queue
		// index i+1 for send queue
		//
		// as it is wanted by the network network device.
		// see Virtio specification v1.1. - 5.1.2
		// Assure that we have always an even number of queues (i.e. pairs of queues).
		assert_eq!(self.num_vqs % 2, 0);

		for i in 0..(self.num_vqs / 2) {
			if self
				.dev_cfg
				.features
				.is_feature(Features::VIRTIO_F_RING_PACKED)
			{
				let vq = PackedVq::new(
					&mut self.com_cfg,
					&self.notif_cfg,
					VqSize::from(VIRTIO_MAX_QUEUE_SIZE),
					VqIndex::from(2 * i),
					VirtioF::from_bits_retain(u64::from(self.dev_cfg.features).into()),
				)
				.unwrap();
				// Interrupt for receiving packets is wanted
				vq.enable_notifs();

				self.recv_vqs.add(Rc::from(vq), &self.dev_cfg);

				let vq = PackedVq::new(
					&mut self.com_cfg,
					&self.notif_cfg,
					VqSize::from(VIRTIO_MAX_QUEUE_SIZE),
					VqIndex::from(2 * i + 1),
					VirtioF::from_bits_retain(u64::from(self.dev_cfg.features).into()),
				)
				.unwrap();
				// Interrupt for comunicating that a sended packet left, is not needed
				vq.disable_notifs();

				self.send_vqs.add(Rc::from(vq), &self.dev_cfg);
			} else {
				let vq = SplitVq::new(
					&mut self.com_cfg,
					&self.notif_cfg,
					VqSize::from(VIRTIO_MAX_QUEUE_SIZE),
					VqIndex::from(2 * i),
					VirtioF::from_bits_retain(u64::from(self.dev_cfg.features).into()),
				)
				.unwrap();
				// Interrupt for receiving packets is wanted
				vq.enable_notifs();

				self.recv_vqs.add(Rc::from(vq), &self.dev_cfg);

				let vq = SplitVq::new(
					&mut self.com_cfg,
					&self.notif_cfg,
					VqSize::from(VIRTIO_MAX_QUEUE_SIZE),
					VqIndex::from(2 * i + 1),
					VirtioF::from_bits_retain(u64::from(self.dev_cfg.features).into()),
				)
				.unwrap();
				// Interrupt for comunicating that a sended packet left, is not needed
				vq.disable_notifs();

				self.send_vqs.add(Rc::from(vq), &self.dev_cfg);
			}
		}

		Ok(())
	}
}

pub mod constants {
	use alloc::vec::Vec;
	use core::ops::{BitAnd, BitAndAssign, BitOr, BitOrAssign};

	use zerocopy::AsBytes;

	pub use super::error::VirtioNetError;

	// Configuration constants
	pub const MAX_NUM_VQ: u16 = 2;

	/// Enum containing Virtios netword header flags
	///
	/// See Virtio specification v1.1. - 5.1.6
	#[allow(dead_code, non_camel_case_types)]
	#[derive(AsBytes, Copy, Clone, Debug)]
	#[repr(u8)]
	pub enum NetHdrFlag {
		/// No further information
		VIRTIO_NET_HDR_F_NONE = 0,
		/// use csum_start, csum_offset
		VIRTIO_NET_HDR_F_NEEDS_CSUM = 1,
		/// csum is valid
		VIRTIO_NET_HDR_F_DATA_VALID = 2,
		/// reports number of coalesced TCP segments
		VIRTIO_NET_HDR_F_RSC_INFO = 4,
	}

	impl From<NetHdrFlag> for u8 {
		fn from(val: NetHdrFlag) -> Self {
			match val {
				NetHdrFlag::VIRTIO_NET_HDR_F_NONE => 0,
				NetHdrFlag::VIRTIO_NET_HDR_F_NEEDS_CSUM => 1,
				NetHdrFlag::VIRTIO_NET_HDR_F_DATA_VALID => 2,
				NetHdrFlag::VIRTIO_NET_HDR_F_RSC_INFO => 4,
			}
		}
	}

	/// Enum containing Virtios netword GSO types
	///
	/// See Virtio specification v1.1. - 5.1.6
	#[allow(dead_code, non_camel_case_types)]
	#[derive(AsBytes, Copy, Clone, Debug)]
	#[repr(u8)]
	pub enum NetHdrGSO {
		/// not a GSO frame
		VIRTIO_NET_HDR_GSO_NONE = 0,
		/// GSO frame, IPv4 TCP (TSO)
		VIRTIO_NET_HDR_GSO_TCPV4 = 1,
		/// GSO frame, IPv4 UDP (UFO)
		VIRTIO_NET_HDR_GSO_UDP = 3,
		/// GSO frame, IPv6 TCP
		VIRTIO_NET_HDR_GSO_TCPV6 = 4,
		/// TCP has ECN set
		VIRTIO_NET_HDR_GSO_ECN = 0x80,
	}

	impl From<NetHdrGSO> for u8 {
		fn from(val: NetHdrGSO) -> Self {
			match val {
				NetHdrGSO::VIRTIO_NET_HDR_GSO_NONE => 0,
				NetHdrGSO::VIRTIO_NET_HDR_GSO_TCPV4 => 1,
				NetHdrGSO::VIRTIO_NET_HDR_GSO_UDP => 3,
				NetHdrGSO::VIRTIO_NET_HDR_GSO_TCPV6 => 4,
				NetHdrGSO::VIRTIO_NET_HDR_GSO_ECN => 0x80,
			}
		}
	}

	/// Enum contains virtio's network device features and general features of Virtio.
	///
	/// See Virtio specification v1.1. - 5.1.3
	///
	/// See Virtio specification v1.1. - 6
	//
	// WARN: In case the enum is changed, the static function of features `into_features(feature: u64) ->
	// Option<Vec<Features>>` must also be adjusted to return a correct vector of features.
	#[allow(dead_code, non_camel_case_types)]
	#[derive(Copy, Clone, Debug)]
	#[repr(u64)]
	pub enum Features {
		VIRTIO_NET_F_CSUM = 1 << 0,
		VIRTIO_NET_F_GUEST_CSUM = 1 << 1,
		VIRTIO_NET_F_CTRL_GUEST_OFFLOADS = 1 << 2,
		VIRTIO_NET_F_MTU = 1 << 3,
		VIRTIO_NET_F_MAC = 1 << 5,
		VIRTIO_NET_F_GUEST_TSO4 = 1 << 7,
		VIRTIO_NET_F_GUEST_TSO6 = 1 << 8,
		VIRTIO_NET_F_GUEST_ECN = 1 << 9,
		VIRTIO_NET_F_GUEST_UFO = 1 << 10,
		VIRTIO_NET_F_HOST_TSO4 = 1 << 11,
		VIRTIO_NET_F_HOST_TSO6 = 1 << 12,
		VIRTIO_NET_F_HOST_ECN = 1 << 13,
		VIRTIO_NET_F_HOST_UFO = 1 << 14,
		VIRTIO_NET_F_MRG_RXBUF = 1 << 15,
		VIRTIO_NET_F_STATUS = 1 << 16,
		VIRTIO_NET_F_CTRL_VQ = 1 << 17,
		VIRTIO_NET_F_CTRL_RX = 1 << 18,
		VIRTIO_NET_F_CTRL_VLAN = 1 << 19,
		VIRTIO_NET_F_GUEST_ANNOUNCE = 1 << 21,
		VIRTIO_NET_F_MQ = 1 << 22,
		VIRTIO_NET_F_CTRL_MAC_ADDR = 1 << 23,
		VIRTIO_F_RING_INDIRECT_DESC = 1 << 28,
		VIRTIO_F_RING_EVENT_IDX = 1 << 29,
		VIRTIO_F_VERSION_1 = 1 << 32,
		VIRTIO_F_ACCESS_PLATFORM = 1 << 33,
		VIRTIO_F_RING_PACKED = 1 << 34,
		VIRTIO_F_IN_ORDER = 1 << 35,
		VIRTIO_F_ORDER_PLATFORM = 1 << 36,
		VIRTIO_F_SR_IOV = 1 << 37,
		VIRTIO_F_NOTIFICATION_DATA = 1 << 38,
		VIRTIO_NET_F_GUEST_HDRLEN = 1 << 59,
		VIRTIO_NET_F_RSC_EXT = 1 << 61,
		VIRTIO_NET_F_STANDBY = 1 << 62,
		// INTERNAL DOCUMENTATION TO KNOW WHICH FEATURES HAVE REQUIREMENTS
		//
		// 5.1.3.1 Feature bit requirements
		// Some networking feature bits require other networking feature bits (see 2.2.1):
		// VIRTIO_NET_F_GUEST_TSO4 Requires VIRTIO_NET_F_GUEST_CSUM.
		// VIRTIO_NET_F_GUEST_TSO6 Requires VIRTIO_NET_F_GUEST_CSUM.
		// VIRTIO_NET_F_GUEST_ECN Requires VIRTIO_NET_F_GUEST_TSO4orVIRTIO_NET_F_GUEST_TSO6.
		// VIRTIO_NET_F_GUEST_UFO Requires VIRTIO_NET_F_GUEST_CSUM.
		// VIRTIO_NET_F_HOST_TSO4 Requires VIRTIO_NET_F_CSUM.
		// VIRTIO_NET_F_HOST_TSO6 Requires VIRTIO_NET_F_CSUM.
		// VIRTIO_NET_F_HOST_ECN Requires VIRTIO_NET_F_HOST_TSO4 or VIRTIO_NET_F_HOST_TSO6.
		// VIRTIO_NET_F_HOST_UFO Requires VIRTIO_NET_F_CSUM.
		// VIRTIO_NET_F_CTRL_RX Requires VIRTIO_NET_F_CTRL_VQ.
		// VIRTIO_NET_F_CTRL_VLAN Requires VIRTIO_NET_F_CTRL_VQ.
		// VIRTIO_NET_F_GUEST_ANNOUNCE Requires VIRTIO_NET_F_CTRL_VQ.
		// VIRTIO_NET_F_MQ Requires VIRTIO_NET_F_CTRL_VQ.
		// VIRTIO_NET_F_CTRL_MAC_ADDR Requires VIRTIO_NET_F_CTRL_VQ.
		// VIRTIO_NET_F_RSC_EXT Requires VIRTIO_NET_F_HOST_TSO4 or VIRTIO_NET_F_HOST_TSO6.
	}

	impl From<Features> for u64 {
		fn from(val: Features) -> Self {
			match val {
				Features::VIRTIO_NET_F_CSUM => 1 << 0,
				Features::VIRTIO_NET_F_GUEST_CSUM => 1 << 1,
				Features::VIRTIO_NET_F_CTRL_GUEST_OFFLOADS => 1 << 2,
				Features::VIRTIO_NET_F_MTU => 1 << 3,
				Features::VIRTIO_NET_F_MAC => 1 << 5,
				Features::VIRTIO_NET_F_GUEST_TSO4 => 1 << 7,
				Features::VIRTIO_NET_F_GUEST_TSO6 => 1 << 8,
				Features::VIRTIO_NET_F_GUEST_ECN => 1 << 9,
				Features::VIRTIO_NET_F_GUEST_UFO => 1 << 10,
				Features::VIRTIO_NET_F_HOST_TSO4 => 1 << 11,
				Features::VIRTIO_NET_F_HOST_TSO6 => 1 << 12,
				Features::VIRTIO_NET_F_HOST_ECN => 1 << 13,
				Features::VIRTIO_NET_F_HOST_UFO => 1 << 14,
				Features::VIRTIO_NET_F_MRG_RXBUF => 1 << 15,
				Features::VIRTIO_NET_F_STATUS => 1 << 16,
				Features::VIRTIO_NET_F_CTRL_VQ => 1 << 17,
				Features::VIRTIO_NET_F_CTRL_RX => 1 << 18,
				Features::VIRTIO_NET_F_CTRL_VLAN => 1 << 19,
				Features::VIRTIO_NET_F_GUEST_ANNOUNCE => 1 << 21,
				Features::VIRTIO_NET_F_MQ => 1 << 22,
				Features::VIRTIO_NET_F_CTRL_MAC_ADDR => 1 << 23,
				Features::VIRTIO_F_RING_INDIRECT_DESC => 1 << 28,
				Features::VIRTIO_F_RING_EVENT_IDX => 1 << 29,
				Features::VIRTIO_F_VERSION_1 => 1 << 32,
				Features::VIRTIO_F_ACCESS_PLATFORM => 1 << 33,
				Features::VIRTIO_F_RING_PACKED => 1 << 34,
				Features::VIRTIO_F_IN_ORDER => 1 << 35,
				Features::VIRTIO_F_ORDER_PLATFORM => 1 << 36,
				Features::VIRTIO_F_SR_IOV => 1 << 37,
				Features::VIRTIO_F_NOTIFICATION_DATA => 1 << 38,
				Features::VIRTIO_NET_F_GUEST_HDRLEN => 1 << 59,
				Features::VIRTIO_NET_F_RSC_EXT => 1 << 61,
				Features::VIRTIO_NET_F_STANDBY => 1 << 62,
			}
		}
	}

	impl BitOr for Features {
		type Output = u64;

		fn bitor(self, rhs: Self) -> Self::Output {
			u64::from(self) | u64::from(rhs)
		}
	}

	impl BitOr<Features> for u64 {
		type Output = u64;

		fn bitor(self, rhs: Features) -> Self::Output {
			self | u64::from(rhs)
		}
	}

	impl BitOrAssign<Features> for u64 {
		fn bitor_assign(&mut self, rhs: Features) {
			*self |= u64::from(rhs);
		}
	}

	impl BitAnd for Features {
		type Output = u64;

		fn bitand(self, rhs: Features) -> Self::Output {
			u64::from(self) & u64::from(rhs)
		}
	}

	impl BitAnd<Features> for u64 {
		type Output = u64;

		fn bitand(self, rhs: Features) -> Self::Output {
			self & u64::from(rhs)
		}
	}

	impl BitAndAssign<Features> for u64 {
		fn bitand_assign(&mut self, rhs: Features) {
			*self &= u64::from(rhs);
		}
	}

	impl core::fmt::Display for Features {
		fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
			match *self {
				Features::VIRTIO_NET_F_CSUM => write!(f, "VIRTIO_NET_F_CSUM"),
				Features::VIRTIO_NET_F_GUEST_CSUM => write!(f, "VIRTIO_NET_F_GUEST_CSUM"),
				Features::VIRTIO_NET_F_CTRL_GUEST_OFFLOADS => {
					write!(f, "VIRTIO_NET_F_CTRL_GUEST_OFFLOADS")
				}
				Features::VIRTIO_NET_F_MTU => write!(f, "VIRTIO_NET_F_MTU"),
				Features::VIRTIO_NET_F_MAC => write!(f, "VIRTIO_NET_F_MAC"),
				Features::VIRTIO_NET_F_GUEST_TSO4 => write!(f, "VIRTIO_NET_F_GUEST_TSO4"),
				Features::VIRTIO_NET_F_GUEST_TSO6 => write!(f, "VIRTIO_NET_F_GUEST_TSO6"),
				Features::VIRTIO_NET_F_GUEST_ECN => write!(f, "VIRTIO_NET_F_GUEST_ECN"),
				Features::VIRTIO_NET_F_GUEST_UFO => write!(f, "VIRTIO_NET_FGUEST_UFO"),
				Features::VIRTIO_NET_F_HOST_TSO4 => write!(f, "VIRTIO_NET_F_HOST_TSO4"),
				Features::VIRTIO_NET_F_HOST_TSO6 => write!(f, "VIRTIO_NET_F_HOST_TSO6"),
				Features::VIRTIO_NET_F_HOST_ECN => write!(f, "VIRTIO_NET_F_HOST_ECN"),
				Features::VIRTIO_NET_F_HOST_UFO => write!(f, "VIRTIO_NET_F_HOST_UFO"),
				Features::VIRTIO_NET_F_MRG_RXBUF => write!(f, "VIRTIO_NET_F_MRG_RXBUF"),
				Features::VIRTIO_NET_F_STATUS => write!(f, "VIRTIO_NET_F_STATUS"),
				Features::VIRTIO_NET_F_CTRL_VQ => write!(f, "VIRTIO_NET_F_CTRL_VQ"),
				Features::VIRTIO_NET_F_CTRL_RX => write!(f, "VIRTIO_NET_F_CTRL_RX"),
				Features::VIRTIO_NET_F_CTRL_VLAN => write!(f, "VIRTIO_NET_F_CTRL_VLAN"),
				Features::VIRTIO_NET_F_GUEST_ANNOUNCE => write!(f, "VIRTIO_NET_F_GUEST_ANNOUNCE"),
				Features::VIRTIO_NET_F_MQ => write!(f, "VIRTIO_NET_F_MQ"),
				Features::VIRTIO_NET_F_CTRL_MAC_ADDR => write!(f, "VIRTIO_NET_F_CTRL_MAC_ADDR"),
				Features::VIRTIO_F_RING_INDIRECT_DESC => write!(f, "VIRTIO_F_RING_INDIRECT_DESC"),
				Features::VIRTIO_F_RING_EVENT_IDX => write!(f, "VIRTIO_F_RING_EVENT_IDX"),
				Features::VIRTIO_F_VERSION_1 => write!(f, "VIRTIO_F_VERSION_1"),
				Features::VIRTIO_F_ACCESS_PLATFORM => write!(f, "VIRTIO_F_ACCESS_PLATFORM"),
				Features::VIRTIO_F_RING_PACKED => write!(f, "VIRTIO_F_RING_PACKED"),
				Features::VIRTIO_F_IN_ORDER => write!(f, "VIRTIO_F_IN_ORDER"),
				Features::VIRTIO_F_ORDER_PLATFORM => write!(f, "VIRTIO_F_ORDER_PLATFORM"),
				Features::VIRTIO_F_SR_IOV => write!(f, "VIRTIO_F_SR_IOV"),
				Features::VIRTIO_F_NOTIFICATION_DATA => write!(f, "VIRTIO_F_NOTIFICATION_DATA"),
				Features::VIRTIO_NET_F_GUEST_HDRLEN => write!(f, "VIRTIO_NET_F_GUEST_HDRLEN"),
				Features::VIRTIO_NET_F_RSC_EXT => write!(f, "VIRTIO_NET_F_RSC_EXT"),
				Features::VIRTIO_NET_F_STANDBY => write!(f, "VIRTIO_NET_F_STANDBY"),
			}
		}
	}

	impl Features {
		/// Return a vector of [Features] for a given input of a u64 representation.
		///
		/// INFO: In case the FEATURES enum is changed, this function MUST also be adjusted to the new set!
		//
		// Really UGLY function, but currently the most convenienvt one to reduce the set of features for the driver easily!
		pub fn from_set(feature_set: FeatureSet) -> Option<Vec<Features>> {
			let mut features_vec: Vec<Features> = Vec::new();
			let features = feature_set.0;

			if features & (1 << 0) != 0 {
				features_vec.push(Features::VIRTIO_NET_F_CSUM)
			}
			if features & (1 << 1) != 0 {
				features_vec.push(Features::VIRTIO_NET_F_GUEST_CSUM)
			}
			if features & (1 << 2) != 0 {
				features_vec.push(Features::VIRTIO_NET_F_CTRL_GUEST_OFFLOADS)
			}
			if features & (1 << 3) != 0 {
				features_vec.push(Features::VIRTIO_NET_F_MTU)
			}
			if features & (1 << 5) != 0 {
				features_vec.push(Features::VIRTIO_NET_F_MAC)
			}
			if features & (1 << 7) != 0 {
				features_vec.push(Features::VIRTIO_NET_F_GUEST_TSO4)
			}
			if features & (1 << 8) != 0 {
				features_vec.push(Features::VIRTIO_NET_F_GUEST_TSO6)
			}
			if features & (1 << 9) != 0 {
				features_vec.push(Features::VIRTIO_NET_F_GUEST_ECN)
			}
			if features & (1 << 10) != 0 {
				features_vec.push(Features::VIRTIO_NET_F_GUEST_UFO)
			}
			if features & (1 << 11) != 0 {
				features_vec.push(Features::VIRTIO_NET_F_HOST_TSO4)
			}
			if features & (1 << 12) != 0 {
				features_vec.push(Features::VIRTIO_NET_F_HOST_TSO6)
			}
			if features & (1 << 13) != 0 {
				features_vec.push(Features::VIRTIO_NET_F_HOST_ECN)
			}
			if features & (1 << 14) != 0 {
				features_vec.push(Features::VIRTIO_NET_F_HOST_UFO)
			}
			if features & (1 << 15) != 0 {
				features_vec.push(Features::VIRTIO_NET_F_MRG_RXBUF)
			}
			if features & (1 << 16) != 0 {
				features_vec.push(Features::VIRTIO_NET_F_STATUS)
			}
			if features & (1 << 17) != 0 {
				features_vec.push(Features::VIRTIO_NET_F_CTRL_VQ)
			}
			if features & (1 << 18) != 0 {
				features_vec.push(Features::VIRTIO_NET_F_CTRL_RX)
			}
			if features & (1 << 19) != 0 {
				features_vec.push(Features::VIRTIO_NET_F_CTRL_VLAN)
			}
			if features & (1 << 21) != 0 {
				features_vec.push(Features::VIRTIO_NET_F_GUEST_ANNOUNCE)
			}
			if features & (1 << 22) != 0 {
				features_vec.push(Features::VIRTIO_NET_F_MQ)
			}
			if features & (1 << 23) != 0 {
				features_vec.push(Features::VIRTIO_NET_F_CTRL_MAC_ADDR)
			}
			if features & (1 << 28) != 0 {
				features_vec.push(Features::VIRTIO_F_RING_INDIRECT_DESC)
			}
			if features & (1 << 29) != 0 {
				features_vec.push(Features::VIRTIO_F_RING_EVENT_IDX)
			}
			if features & (1 << 32) != 0 {
				features_vec.push(Features::VIRTIO_F_VERSION_1)
			}
			if features & (1 << 33) != 0 {
				features_vec.push(Features::VIRTIO_F_ACCESS_PLATFORM)
			}
			if features & (1 << 34) != 0 {
				features_vec.push(Features::VIRTIO_F_RING_PACKED)
			}
			if features & (1 << 35) != 0 {
				features_vec.push(Features::VIRTIO_F_IN_ORDER)
			}
			if features & (1 << 36) != 0 {
				features_vec.push(Features::VIRTIO_F_ORDER_PLATFORM)
			}
			if features & (1 << 37) != 0 {
				features_vec.push(Features::VIRTIO_F_SR_IOV)
			}
			if features & (1 << 38) != 0 {
				features_vec.push(Features::VIRTIO_F_NOTIFICATION_DATA)
			}
			if features & (1 << 59) != 0 {
				features_vec.push(Features::VIRTIO_NET_F_GUEST_HDRLEN)
			}
			if features & (1 << 61) != 0 {
				features_vec.push(Features::VIRTIO_NET_F_RSC_EXT)
			}
			if features & (1 << 62) != 0 {
				features_vec.push(Features::VIRTIO_NET_F_STANDBY)
			}

			if features_vec.is_empty() {
				None
			} else {
				Some(features_vec)
			}
		}
	}

	/// Enum contains virtio's network device status
	/// indiacted in the status field of the device's
	/// configuration structure.
	///
	/// See Virtio specification v1.1. - 5.1.4
	#[allow(dead_code, non_camel_case_types)]
	#[derive(Copy, Clone, Debug)]
	#[repr(u16)]
	pub enum Status {
		VIRTIO_NET_S_LINK_UP = 1 << 0,
		VIRTIO_NET_S_ANNOUNCE = 1 << 1,
	}

	impl From<Status> for u16 {
		fn from(stat: Status) -> Self {
			match stat {
				Status::VIRTIO_NET_S_LINK_UP => 1,
				Status::VIRTIO_NET_S_ANNOUNCE => 2,
			}
		}
	}

	/// FeatureSet is new type whicih holds features for virito network devices indicated by the virtio specification
	/// v1.1. - 5.1.3. and all General Features defined in Virtio specification v1.1. - 6
	/// wrapping a u64.
	///
	/// The main functionality of this type are functions implemented on it.
	#[derive(Debug, Copy, Clone, PartialOrd, PartialEq, Eq)]
	pub struct FeatureSet(u64);

	impl BitOr for FeatureSet {
		type Output = FeatureSet;

		fn bitor(self, rhs: Self) -> Self::Output {
			FeatureSet(self.0 | rhs.0)
		}
	}

	impl BitOr<FeatureSet> for u64 {
		type Output = u64;

		fn bitor(self, rhs: FeatureSet) -> Self::Output {
			self | u64::from(rhs)
		}
	}

	impl BitOrAssign<FeatureSet> for u64 {
		fn bitor_assign(&mut self, rhs: FeatureSet) {
			*self |= u64::from(rhs);
		}
	}

	impl BitOrAssign<Features> for FeatureSet {
		fn bitor_assign(&mut self, rhs: Features) {
			self.0 = self.0 | u64::from(rhs);
		}
	}

	impl BitAnd for FeatureSet {
		type Output = FeatureSet;

		fn bitand(self, rhs: FeatureSet) -> Self::Output {
			FeatureSet(self.0 & rhs.0)
		}
	}

	impl BitAnd<FeatureSet> for u64 {
		type Output = u64;

		fn bitand(self, rhs: FeatureSet) -> Self::Output {
			self & u64::from(rhs)
		}
	}

	impl BitAndAssign<FeatureSet> for u64 {
		fn bitand_assign(&mut self, rhs: FeatureSet) {
			*self &= u64::from(rhs);
		}
	}

	impl From<FeatureSet> for u64 {
		fn from(feature_set: FeatureSet) -> Self {
			feature_set.0
		}
	}

	impl FeatureSet {
		/// Checks if a given set of features is compatible and adheres to the
		/// specfification v1.1. - 5.1.3.1
		/// Upon an error returns the incompatible set of features by the
		/// [FeatureRequirementsNotMet](super::error::VirtioNetError) error value, which
		/// wraps the u64 indicating the feature set.
		///
		/// INFO: Iterates twice over the vector of features.
		pub fn check_features(features: &[Features]) -> Result<(), VirtioNetError> {
			let mut feature_bits = 0u64;

			for feature in features.iter() {
				feature_bits |= *feature;
			}

			for feature in features {
				match feature {
					Features::VIRTIO_NET_F_CSUM => continue,
					Features::VIRTIO_NET_F_GUEST_CSUM => continue,
					Features::VIRTIO_NET_F_CTRL_GUEST_OFFLOADS => continue,
					Features::VIRTIO_NET_F_MTU => continue,
					Features::VIRTIO_NET_F_MAC => continue,
					Features::VIRTIO_NET_F_GUEST_TSO4 => {
						if feature_bits & Features::VIRTIO_NET_F_GUEST_CSUM != 0 {
							continue;
						} else {
							return Err(VirtioNetError::FeatureRequirementsNotMet(FeatureSet(
								feature_bits,
							)));
						}
					}
					Features::VIRTIO_NET_F_GUEST_TSO6 => {
						if feature_bits & Features::VIRTIO_NET_F_GUEST_CSUM != 0 {
							continue;
						} else {
							return Err(VirtioNetError::FeatureRequirementsNotMet(FeatureSet(
								feature_bits,
							)));
						}
					}
					Features::VIRTIO_NET_F_GUEST_ECN => {
						if feature_bits
							& (Features::VIRTIO_NET_F_GUEST_TSO4
								| Features::VIRTIO_NET_F_GUEST_TSO6)
							!= 0
						{
							continue;
						} else {
							return Err(VirtioNetError::FeatureRequirementsNotMet(FeatureSet(
								feature_bits,
							)));
						}
					}
					Features::VIRTIO_NET_F_GUEST_UFO => {
						if feature_bits & Features::VIRTIO_NET_F_GUEST_CSUM != 0 {
							continue;
						} else {
							return Err(VirtioNetError::FeatureRequirementsNotMet(FeatureSet(
								feature_bits,
							)));
						}
					}
					Features::VIRTIO_NET_F_HOST_TSO4 => {
						if feature_bits & Features::VIRTIO_NET_F_CSUM != 0 {
							continue;
						} else {
							return Err(VirtioNetError::FeatureRequirementsNotMet(FeatureSet(
								feature_bits,
							)));
						}
					}
					Features::VIRTIO_NET_F_HOST_TSO6 => {
						if feature_bits & Features::VIRTIO_NET_F_CSUM != 0 {
							continue;
						} else {
							return Err(VirtioNetError::FeatureRequirementsNotMet(FeatureSet(
								feature_bits,
							)));
						}
					}
					Features::VIRTIO_NET_F_HOST_ECN => {
						if feature_bits
							& (Features::VIRTIO_NET_F_HOST_TSO4 | Features::VIRTIO_NET_F_HOST_TSO6)
							!= 0
						{
							continue;
						} else {
							return Err(VirtioNetError::FeatureRequirementsNotMet(FeatureSet(
								feature_bits,
							)));
						}
					}
					Features::VIRTIO_NET_F_HOST_UFO => {
						if feature_bits & Features::VIRTIO_NET_F_CSUM != 0 {
							continue;
						} else {
							return Err(VirtioNetError::FeatureRequirementsNotMet(FeatureSet(
								feature_bits,
							)));
						}
					}
					Features::VIRTIO_NET_F_MRG_RXBUF => continue,
					Features::VIRTIO_NET_F_STATUS => continue,
					Features::VIRTIO_NET_F_CTRL_VQ => continue,
					Features::VIRTIO_NET_F_CTRL_RX => {
						if feature_bits & Features::VIRTIO_NET_F_CTRL_VQ != 0 {
							continue;
						} else {
							return Err(VirtioNetError::FeatureRequirementsNotMet(FeatureSet(
								feature_bits,
							)));
						}
					}
					Features::VIRTIO_NET_F_CTRL_VLAN => {
						if feature_bits & Features::VIRTIO_NET_F_CTRL_VQ != 0 {
							continue;
						} else {
							return Err(VirtioNetError::FeatureRequirementsNotMet(FeatureSet(
								feature_bits,
							)));
						}
					}
					Features::VIRTIO_NET_F_GUEST_ANNOUNCE => {
						if feature_bits & Features::VIRTIO_NET_F_CTRL_VQ != 0 {
							continue;
						} else {
							return Err(VirtioNetError::FeatureRequirementsNotMet(FeatureSet(
								feature_bits,
							)));
						}
					}
					Features::VIRTIO_NET_F_MQ => {
						if feature_bits & Features::VIRTIO_NET_F_CTRL_VQ != 0 {
							continue;
						} else {
							return Err(VirtioNetError::FeatureRequirementsNotMet(FeatureSet(
								feature_bits,
							)));
						}
					}
					Features::VIRTIO_NET_F_CTRL_MAC_ADDR => {
						if feature_bits & Features::VIRTIO_NET_F_CTRL_VQ != 0 {
							continue;
						} else {
							return Err(VirtioNetError::FeatureRequirementsNotMet(FeatureSet(
								feature_bits,
							)));
						}
					}
					Features::VIRTIO_NET_F_GUEST_HDRLEN => continue,
					Features::VIRTIO_NET_F_RSC_EXT => {
						if feature_bits
							& (Features::VIRTIO_NET_F_HOST_TSO4 | Features::VIRTIO_NET_F_HOST_TSO6)
							!= 0
						{
							continue;
						} else {
							return Err(VirtioNetError::FeatureRequirementsNotMet(FeatureSet(
								feature_bits,
							)));
						}
					}
					Features::VIRTIO_NET_F_STANDBY => continue,
					Features::VIRTIO_F_RING_INDIRECT_DESC => continue,
					Features::VIRTIO_F_RING_EVENT_IDX => continue,
					Features::VIRTIO_F_VERSION_1 => continue,
					Features::VIRTIO_F_ACCESS_PLATFORM => continue,
					Features::VIRTIO_F_RING_PACKED => continue,
					Features::VIRTIO_F_IN_ORDER => continue,
					Features::VIRTIO_F_ORDER_PLATFORM => continue,
					Features::VIRTIO_F_SR_IOV => continue,
					Features::VIRTIO_F_NOTIFICATION_DATA => continue,
				}
			}

			Ok(())
		}

		/// Checks if a given feature is set.
		pub fn is_feature(self, feature: Features) -> bool {
			self.0 & feature != 0
		}

		/// Sets features contained in features to true.
		///
		/// WARN: Features should be checked before using this function via the [`FeatureSet::check_features`] function.
		pub fn set_features(&mut self, features: &[Features]) {
			for feature in features {
				self.0 |= *feature;
			}
		}

		/// Returns a new instance of (FeatureSet)[FeatureSet] with all features
		/// initialized to false.
		pub fn new(val: u64) -> Self {
			FeatureSet(val)
		}
	}
}

/// Error module of virtios network driver. Containing the (VirtioNetError)[VirtioNetError]
/// enum.
pub mod error {
	use super::constants::FeatureSet;
	/// Network drivers error enum.
	#[derive(Debug, Copy, Clone)]
	pub enum VirtioNetError {
		#[cfg(feature = "pci")]
		NoDevCfg(u16),
		#[cfg(feature = "pci")]
		NoComCfg(u16),
		#[cfg(feature = "pci")]
		NoIsrCfg(u16),
		#[cfg(feature = "pci")]
		NoNotifCfg(u16),
		FailFeatureNeg(u16),
		/// Set of features does not adhere to the requirements of features
		/// indicated by the specification
		FeatureRequirementsNotMet(FeatureSet),
		/// The first u64 contains the feature bits wanted by the driver.
		/// but which are incompatible with the device feature set, second u64.
		IncompatibleFeatureSets(FeatureSet, FeatureSet),
	}
}
