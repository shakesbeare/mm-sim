use bevy::math;
use bevy::prelude::*;
use rand::Rng as _;
use rand_distr::Distribution as _;
use rand_distr::Normal;

#[derive(Resource, Clone, Debug, PartialEq, PartialOrd, Eq, Ord)]
pub struct SimTime {
    ticks: usize,
}

impl SimTime {
    #[inline]
    pub fn new() -> Self {
        Self { ticks: 0 }
    }

    #[inline]
    pub fn tick(&mut self) {
        self.ticks += 1;
    }

    /// Helper function to return the date time with no offset
    #[inline]
    pub fn base_time(&self) -> DateTime {
        self.datetime_from_offset(0)
    }

    /// Returns a date time object from the provided offset in hours
    /// Offsets are taken modulo 24 before calculations are done.
    #[inline]
    pub fn datetime_from_offset(&self, offset: usize) -> DateTime {
        let offset = offset % 24;
        let second = self.ticks + (offset * 60 * 60);
        let (second, minute) = (second % 60, second / 60);
        let (minute, hour) = (minute % 60, minute / 60);
        let (hour, day) = (hour % 24, hour / 24);
        let (day, month) = (day % 30, day / 30);
        let (month, year) = (month % 12, month / 12);

        DateTime {
            offset,
            year,
            month,
            day,
            hour,
            minute,
            second,
        }
    }

    #[inline]
    pub fn elapsed_secs(&self) -> usize {
        self.ticks
    }

    #[inline]
    pub fn elapsed_mins(&self) -> f64 {
        self.ticks as f64 / 60.0
    }

    #[inline]
    pub fn elapsed_hours(&self) -> f64 {
        self.elapsed_mins() / 60.0
    }

    #[inline]
    pub fn elapsed_days(&self) -> f64 {
        self.elapsed_hours() / 24.0
    }

    #[inline]
    pub fn elapsed_months(&self) -> f64 {
        self.elapsed_days() / 30.0
    }

    #[inline]
    pub fn elapsed_years(&self) -> f64 {
        self.elapsed_days() / 360.0
    }
}

impl Default for SimTime {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Eq, Ord)]
pub struct DateTime {
    offset: usize,
    year: usize,
    month: usize,
    day: usize,
    hour: usize,
    minute: usize,
    second: usize,
}

impl DateTime {
    /// Returns a value between 0.0 and 1.0 representing how likely it is that someone at this time
    /// would be online.
    ///
    /// Accounts for working hours, school schedules, and night time.
    ///
    /// This function abstracts a lot of behavior and may need to end up being more detailed in the
    /// future.
    pub fn likelihood_to_play(&self) -> f64 {
        let time_component = {
            // x is hour, y is likelihood
            let points: [Vec2; 4] = [
                vec2(0.0, 0.5),
                vec2(4.0, 0.0),
                vec2(17.0, 1.0),
                vec2(24.0, 0.5),
            ];
            let spline = CubicCardinalSpline::new_catmull_rom(points)
                .to_curve()
                .unwrap();
            spline
                .iter_positions(240)
                .nth((self.hourf() * 10.0) as usize).unwrap().y
        };

        let month_component = {
            // x is month, y is likelihood
            let points: [Vec2; 4] = [
                vec2(0.0, 0.5),
                vec2(7.0, 1.0),
                vec2(10.0, 0.0),
                vec2(13.0, 0.5),
            ];
            let spline = CubicCardinalSpline::new_catmull_rom(points)
                .to_curve()
                .unwrap();
            spline
                .iter_positions(120)
                .nth((self.monthf() * 10.0) as usize).unwrap().y
        };

        let mut rng = rand::rng();
        let normal = Normal::new(time_component * month_component, 0.25).unwrap();
        let sample = normal.sample(&mut rng);
        f64::max(sample as f64, 0.0)
    }

    #[inline]
    pub fn hourf(&self) -> f32 {
        self.hour as f32 + (self.minute as f32 / 60.0) + (self.second as f32 / 3600.0)
    }

    #[inline]
    pub fn monthf(&self) -> f32 {
        self.month as f32 + (self.day as f32 / 30.0) + (self.hourf() / 24.0 / 36.0)
    }
}
