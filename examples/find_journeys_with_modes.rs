use chrono::{Local, NaiveDateTime, Timelike};
use ojp_rs::{OJP, PersonalMode, SimplifiedTrip};
use rand::prelude::IndexedRandom;
use std::error::Error;
use std::fmt;

/// The kind of trip to search for: regular public transport, or a monomodal
/// individual-transport alternative (walk, bike, car, ...).
#[derive(Debug, Clone, Copy)]
enum TripKind {
    PublicTransport,
    Individual(PersonalMode),
}

const ALL_KINDS: [TripKind; 7] = [
    TripKind::PublicTransport,
    TripKind::Individual(PersonalMode::Foot),
    TripKind::Individual(PersonalMode::Bicycle),
    TripKind::Individual(PersonalMode::Car),
    TripKind::Individual(PersonalMode::Motorcycle),
    TripKind::Individual(PersonalMode::Truck),
    TripKind::Individual(PersonalMode::Scooter),
];

impl fmt::Display for TripKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TripKind::PublicTransport => f.write_str("public transport"),
            TripKind::Individual(mode) => write!(f, "{mode}"),
        }
    }
}

impl TripKind {
    /// The individual transport modes to request on top of the public-transport
    /// results (none for a plain public-transport search).
    fn it_modes(&self) -> &[PersonalMode] {
        match self {
            TripKind::PublicTransport => &[],
            TripKind::Individual(mode) => std::slice::from_ref(mode),
        }
    }
}

/// Searches a trip of the given kind (`None` if the endpoint returned no matching
/// trip, e.g. for individual modes it does not support or distances it does not
/// cover with that mode).
async fn find_trip_of_kind(
    kind: TripKind,
    from_id: i32,
    to_id: i32,
    date_time: NaiveDateTime,
) -> Result<Option<SimplifiedTrip>, Box<dyn Error>> {
    // The Swiss endpoint honours only one ItModeToCover per request, so the kinds
    // are queried one at a time.
    let trips = OJP::find_trips_with_modes(
        from_id,
        to_id,
        date_time,
        3,
        "OJP-Example",
        "TOKEN",
        kind.it_modes(),
    )
    .await?;

    Ok(match kind {
        // First public-transport trip departing after `date_time`.
        TripKind::PublicTransport => trips.into_iter().next(),
        TripKind::Individual(mode) => trips
            .into_iter()
            .find(|t| t.legs().iter().any(|l| l.mode() == mode.as_str())),
    })
}

/// Given a certain amount of `test_cities`, `number_trips` random stop pairs in these
/// cities are searched and, for each pair, one trip of every kind departing after
/// `date_time` is looked up. The per-kind outcome is returned as-is: found trip,
/// no matching trip, or error.
#[allow(clippy::type_complexity)]
async fn find_trips(
    test_cities: &[&str],
    number_trips: usize,
    date_time: NaiveDateTime,
) -> Result<Vec<(TripKind, Result<Option<SimplifiedTrip>, Box<dyn Error>>)>, Box<dyn Error>> {
    // One stop per city, then random departure/arrival pairs among them.
    let point_refs = OJP::find_locations(test_cities, date_time, 1, "OJP-Example", "TOKEN").await?;
    let points: Vec<i32> = point_refs
        .choose_multiple(&mut rand::rng(), 2 * number_trips)
        .copied()
        .collect();
    if points.len() < 2 * number_trips {
        return Err("found fewer stops than needed for the requested trips".into());
    }
    let (departures, arrivals) = points.split_at(number_trips);

    let mut trips = Vec::new();
    for (&from_id, &to_id) in departures.iter().zip(arrivals.iter()) {
        for kind in ALL_KINDS {
            trips.push((
                kind,
                find_trip_of_kind(kind, from_id, to_id, date_time).await,
            ));
        }
    }
    Ok(trips)
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn Error>> {
    dotenvy::dotenv().ok(); // optional

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

    // Truncated to the whole minute, matching timetable granularity.
    let date_time = Local::now()
        .naive_local()
        .with_second(0)
        .and_then(|dt| dt.with_nanosecond(0))
        .expect("0 is a valid second/nanosecond");
    println!("Departing time: {date_time}");
    let res = find_trips(&test_cities, 3, date_time).await?;
    for (kind, outcome) in res {
        println!("===== {kind} =====");
        match outcome {
            Ok(Some(trip)) => println!("{trip}"),
            // Expected for truck (silently ignored by the Swiss endpoint) and for
            // modes whose distance limits the random pair exceeds (e.g. foot).
            Ok(None) => println!("(no {kind} trip returned)\n"),
            Err(e) => println!("(error searching {kind} trip: {e})\n"),
        }
    }
    Ok(())
}
