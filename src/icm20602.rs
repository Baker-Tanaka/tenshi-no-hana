use embassy_time::{Duration, Timer};
use embedded_hal::i2c::I2c;

const ADDRESS: u8 = 0x68;
const WHO_AM_I: u8 = 0x75;
const EXPECTED_WHO_AM_I: u8 = 0x12;
const PWR_MGMT_1: u8 = 0x6B;
const ACCEL_XOUT_H: u8 = 0x3B;

#[derive(Clone, Copy, Debug, Default)]
pub struct Reading {
    pub acceleration: [f64; 3],
    pub angular_velocity: [f64; 3],
}

pub async fn init<I2C>(i2c: &mut I2C) -> Result<(), ()>
where
    I2C: I2c,
{
    let mut id = [0u8; 1];
    i2c.write_read(ADDRESS, &[WHO_AM_I], &mut id)
        .map_err(|_| ())?;
    if id[0] != EXPECTED_WHO_AM_I {
        return Err(());
    }
    i2c.write(ADDRESS, &[PWR_MGMT_1, 0x01]).map_err(|_| ())?;
    Timer::after(Duration::from_millis(100)).await;
    Ok(())
}

pub fn read<I2C>(i2c: &mut I2C) -> Result<Reading, ()>
where
    I2C: I2c,
{
    let mut raw = [0u8; 14];
    i2c.write_read(ADDRESS, &[ACCEL_XOUT_H], &mut raw)
        .map_err(|_| ())?;

    const ACCEL_SCALE: f64 = 2.0 * 9.80665 / 32768.0;
    const GYRO_SCALE: f64 = 250.0 * core::f64::consts::PI / 180.0 / 32768.0;
    let accel = [
        i16::from_be_bytes([raw[0], raw[1]]) as f64 * ACCEL_SCALE,
        i16::from_be_bytes([raw[2], raw[3]]) as f64 * ACCEL_SCALE,
        i16::from_be_bytes([raw[4], raw[5]]) as f64 * ACCEL_SCALE,
    ];
    let gyro = [
        i16::from_be_bytes([raw[8], raw[9]]) as f64 * GYRO_SCALE,
        i16::from_be_bytes([raw[10], raw[11]]) as f64 * GYRO_SCALE,
        i16::from_be_bytes([raw[12], raw[13]]) as f64 * GYRO_SCALE,
    ];
    Ok(Reading {
        acceleration: accel,
        angular_velocity: gyro,
    })
}
