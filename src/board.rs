use defmt::info;
use embassy_rp::{
    adc::{Adc, Blocking as AdcBlocking, Channel as AdcChannel, Config as AdcConfig},
    bind_interrupts, dma,
    gpio::{Input, Level, Output, Pull},
    i2c::{Blocking, Config as I2cConfig, I2c},
    peripherals::{DMA_CH0, DMA_CH1, I2C0, SPI0},
    pwm::{Config as PwmConfig, Pwm},
    spi::{Async, Config as SpiConfig, Phase, Polarity, Spi},
    Peripherals,
};
use embassy_time::Delay;
use embedded_hal_bus::spi::ExclusiveDevice;
use tenshi_no_hana::drivers::{
    drv8835::{Drv8835, DEFAULT_PWM_TOP},
    hcsr04::Hcsr04,
};

bind_interrupts!(struct Irqs {
    DMA_IRQ_0 => dma::InterruptHandler<DMA_CH0>, dma::InterruptHandler<DMA_CH1>;
});

pub(crate) type WifiSpi = ExclusiveDevice<Spi<'static, SPI0, Async>, Output<'static>, Delay>;
pub(crate) type Imu = I2c<'static, I2C0, Blocking>;

pub(crate) struct AlcoholSensor {
    pub adc: Adc<'static, AdcBlocking>,
    pub channel: AdcChannel<'static>,
}

pub(crate) struct Wifi {
    pub spi: WifiSpi,
    pub handshake: Input<'static>,
    pub data_ready: Input<'static>,
    pub reset: Output<'static>,
}

pub(crate) struct Board {
    pub wifi: Wifi,
    pub imu: Imu,
    pub range: Hcsr04<'static>,
    pub motors: Drv8835<'static>,
    pub alcohol: AlcoholSensor,
}

pub(crate) fn init(p: Peripherals) -> Board {
    let alcohol = AlcoholSensor {
        adc: Adc::new_blocking(p.ADC, AdcConfig::default()),
        channel: AdcChannel::new_pin(p.PIN_28, Pull::None),
    };

    let mut spi_config = SpiConfig::default();
    spi_config.frequency = 10_000_000;
    spi_config.polarity = Polarity::IdleHigh;
    spi_config.phase = Phase::CaptureOnSecondTransition;
    let spi = Spi::new(
        p.SPI0, p.PIN_18, p.PIN_19, p.PIN_16, p.DMA_CH0, p.DMA_CH1, Irqs, spi_config,
    );
    let wifi = Wifi {
        spi: ExclusiveDevice::new(spi, Output::new(p.PIN_17, Level::High), Delay).unwrap(),
        handshake: Input::new(p.PIN_15, Pull::Down),
        data_ready: Input::new(p.PIN_13, Pull::Down),
        reset: Output::new(p.PIN_14, Level::Low),
    };
    let imu = I2c::new_blocking(p.I2C0, p.PIN_5, p.PIN_4, I2cConfig::default());
    let range = Hcsr04::new(
        Output::new(p.PIN_2, Level::Low),
        Input::new(p.PIN_3, Pull::Down),
    );

    let mut pwm_config = PwmConfig::default();
    pwm_config.top = DEFAULT_PWM_TOP;
    pwm_config.compare_b = 0;
    let motors = Drv8835::new(
        Output::new(p.PIN_10, Level::Low),
        Pwm::new_output_b(p.PWM_SLICE5, p.PIN_11, pwm_config.clone()),
        pwm_config.clone(),
        Output::new(p.PIN_12, Level::Low),
        Pwm::new_output_b(p.PWM_SLICE4, p.PIN_9, pwm_config.clone()),
        pwm_config,
    );
    info!("[motor] DRV8835 ready");

    Board {
        wifi,
        imu,
        range,
        motors,
        alcohol,
    }
}
