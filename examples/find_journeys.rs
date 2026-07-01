use chrono::{Local, NaiveDateTime};
use ojp_rs::{OJP, SimplifiedTrip};
use rand::prelude::IndexedRandom;
use std::error::Error;
use tracing::{Level, span, warn};

/// Given a certain amount of `test_cities`, `number_result` random trips departing after
/// `date_time` between arbitrary stops in these cities are searched and returned
pub async fn find_trips(
    test_cities: &[&str],
    number_results: u32,
    date_time: NaiveDateTime,
) -> Result<Vec<SimplifiedTrip>, Box<dyn Error>> {
    dotenvy::dotenv().ok(); // optional
    let point_ref =
        OJP::find_locations(test_cities, date_time, number_results, "OJP-HRDF", "TOKEN").await?;

    let num_travels = number_results as usize;
    let points = point_ref
        .choose_multiple(&mut rand::rng(), 2 * num_travels)
        .copied()
        .collect::<Vec<_>>();
    let (departures, arrivals) = points.split_at(num_travels);

    let number_results = 3;
    let trips = OJP::find_trips(
        departures,
        arrivals,
        date_time,
        number_results,
        "OJP-HRDF",
        "TOKEN",
    )
    .await;
    let (trips, errors): (Vec<_>, Vec<_>) = trips.into_iter().partition(Result::is_ok);
    let trips: Vec<_> = trips.into_iter().map(Result::unwrap).collect();
    let errors: Vec<_> = errors.into_iter().map(Result::unwrap_err).collect();
    for e in errors {
        let span = span!(Level::WARN, "Errors");
        let _guard = span.enter();

        warn!("{e}");
    }
    Ok(trips)
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn Error>> {
    let test_cities = [
        "Zürich",
        "Genève",
        "Basel",
        "Lausanne",
        "Bern",
        "Winterthur",
        "Lucerne",
        "St. Gallen",
        "Lugano",
        "Biel",
        "Thun",
        "Bellinzona",
        "Fribourg",
        "Schaffhausen",
        "Chur",
        "Sion",
        "Zug",
        "Glaris",
    ];
    let date_time = Local::now().naive_local();
    println!("{date_time:?}");
    let res = find_trips(&test_cities, 5, date_time).await?;
    for r in res {
        println!("{}", r);
    }
    Ok(())
}
