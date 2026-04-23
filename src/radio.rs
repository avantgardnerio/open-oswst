use embassy_futures::select::{select, Either};
use esp_idf_svc::hal::gpio::{Gpio10, Gpio11, Gpio12, Gpio13, Gpio14, Gpio8, Gpio9};
use esp_idf_svc::hal::gpio::{Input, Output, PinDriver, Pull};
use esp_idf_svc::hal::spi::config::Config as SpiConfig;
use esp_idf_svc::hal::spi::{SpiDeviceDriver, SpiDriver, SpiDriverConfig, SPI2};
use esp_idf_svc::hal::units::Hertz;
use lora_phy::iv::GenericSx126xInterfaceVariant;
use lora_phy::mod_params::*;
use lora_phy::sx126x::{self, Sx1262, Sx126x, TcxoCtrlVoltage};
use lora_phy::LoRa;
use std::future::Future;

use crate::{RxPacket, RX_CHAN, TX_CHAN};

type Iv<'a> = GenericSx126xInterfaceVariant<PinDriver<'a, Output>, PinDriver<'a, Input>>;
type Radio<'a> =
    LoRa<Sx126x<SpiDeviceDriver<'a, SpiDriver<'a>>, Iv<'a>, Sx1262>, embassy_time::Delay>;

pub struct Peripherals {
    pub spi: SPI2<'static>,
    pub sck: Gpio9<'static>,
    pub mosi: Gpio10<'static>,
    pub miso: Gpio11<'static>,
    pub nss: Gpio8<'static>,
    pub reset: Gpio12<'static>,
    pub dio1: Gpio14<'static>,
    pub busy: Gpio13<'static>,
}

pub async fn init(p: Peripherals) -> impl Future<Output = ()> {
    let spi = SpiDeviceDriver::new_single(
        p.spi,
        p.sck,
        p.mosi,
        Some(p.miso),
        Some(p.nss),
        &SpiDriverConfig::new(),
        &SpiConfig::new().baudrate(Hertz(2_000_000)),
    )
    .unwrap();

    // Degrade pins for type erasure (GenericSx126xInterfaceVariant needs uniform types)
    let reset = PinDriver::output(p.reset.degrade_output()).unwrap();
    let dio1 = PinDriver::input(p.dio1.degrade_input(), Pull::Floating).unwrap();
    let busy = PinDriver::input(p.busy.degrade_input(), Pull::Floating).unwrap();

    let iv = GenericSx126xInterfaceVariant::new(reset, dio1, busy, None, None).unwrap();

    let config = sx126x::Config {
        chip: Sx1262,
        tcxo_ctrl: Some(TcxoCtrlVoltage::Ctrl1V7),
        use_dcdc: true,
        rx_boost: false,
    };

    let mut lora = LoRa::new(Sx126x::new(spi, iv, config), false, embassy_time::Delay)
        .await
        .unwrap();
    log::info!("LoRa radio initialized");

    let mdltn = lora
        .create_modulation_params(
            SpreadingFactor::_7,
            Bandwidth::_125KHz,
            CodingRate::_4_5,
            915_000_000,
        )
        .unwrap();

    let mut tx_params = lora
        .create_tx_packet_params(8, false, true, false, &mdltn)
        .unwrap();

    let rx_params = lora
        .create_rx_packet_params(8, false, 255, true, false, &mdltn)
        .unwrap();

    async move {
        radio_loop(&mut lora, &mdltn, &mut tx_params, &rx_params).await;
    }
}

async fn radio_loop(
    lora: &mut Radio<'_>,
    mdltn: &ModulationParams,
    tx_params: &mut PacketParams,
    rx_params: &PacketParams,
) {
    let mut rx_buf = [0u8; 255];

    loop {
        lora.prepare_for_rx(RxMode::Continuous, mdltn, rx_params)
            .await
            .unwrap();

        match select(lora.rx(rx_params, &mut rx_buf), TX_CHAN.receive()).await {
            Either::First(rx_result) => match rx_result {
                Ok((len, status)) => {
                    let mut data = heapless::Vec::new();
                    let _ = data.extend_from_slice(&rx_buf[..len as usize]);
                    RX_CHAN
                        .send(RxPacket {
                            data,
                            rssi: status.rssi,
                            snr: status.snr,
                        })
                        .await;
                }
                Err(e) => {
                    log::error!("RX error: {:?}", e);
                }
            },
            Either::Second(tx_req) => {
                lora.enter_standby().await.unwrap();
                lora.prepare_for_tx(mdltn, tx_params, 22, &tx_req.data)
                    .await
                    .unwrap();
                lora.tx().await.unwrap();
            }
        }
    }
}
