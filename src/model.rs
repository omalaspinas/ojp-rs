#![allow(dead_code)]
use std::env::VarError;
use std::fmt::Display;
use std::num::ParseIntError;

use chrono::{DateTime, Duration, Local, NaiveDateTime, TimeDelta, Timelike, Utc};
use futures::future::join_all;
use quick_xml::DeError;
use secrecy::SecretString;
use serde::Deserialize;
use thiserror::Error;
use tracing::{Level, span};

use crate::{PersonalMode, RequestBuilder, RequestType, requests::RequestError};

pub fn token(api_key: &str) -> Result<SecretString, OjpError> {
    let t = std::env::var(api_key)?;
    Ok(SecretString::new(t.into()))
}

fn iso_to_uic(iso: &str) -> Option<i32> {
    match iso.to_lowercase().as_str() {
        "fi" => Some(10),
        "ru" => Some(20),
        "by" => Some(21),
        "ua" => Some(22),
        "md" => Some(23),
        "lt" => Some(24),
        "lv" => Some(25),
        "ee" => Some(26),
        "kz" => Some(27),
        "ge" => Some(28),
        "uz" => Some(29),
        "kp" => Some(30),
        "mn" => Some(31),
        "nn" => Some(32),
        "cn" => Some(33),
        "la" => Some(34),
        "cu" => Some(40),
        "al" => Some(41),
        "jp" => Some(42),
        "ba" => Some(44),
        "pl" => Some(51),
        "bg" => Some(52),
        "ro" => Some(53),
        "cz" => Some(54),
        "hu" => Some(55),
        "sk" => Some(56),
        "az" => Some(57),
        "am" => Some(58),
        "kg" => Some(59),
        "ie" => Some(60),
        "kr" => Some(61),
        "me" => Some(62),
        "mk" => Some(65),
        "tj" => Some(66),
        "tm" => Some(67),
        "af" => Some(68),
        "gb" => Some(70),
        "es" => Some(71),
        "rs" => Some(72),
        "gr" => Some(73),
        "se" => Some(74),
        "tr" => Some(75),
        "no" => Some(76),
        "hr" => Some(78),
        "si" => Some(79),
        "de" => Some(80),
        "at" => Some(81),
        "lu" => Some(82),
        "it" => Some(83),
        "nl" => Some(84),
        "ch" => Some(85),
        "dk" => Some(86),
        "fr" => Some(87),
        "be" => Some(88),
        "tz" => Some(89),
        "eg" => Some(90),
        "tn" => Some(91),
        "dz" => Some(92),
        "ma" => Some(93),
        "pt" => Some(94),
        "il" => Some(95),
        "ir" => Some(96),
        "sy" => Some(97),
        "lb" => Some(98),
        "iq" => Some(99),
        _ => None,
    }
}

fn sloid_to_didok(sloid: &str) -> Result<i32, OjpError> {
    // Split SLOID into parts
    let parts: Vec<&str> = sloid.split(':').collect();
    if parts.len() < 4 {
        return Err(OjpError::MalformedSloid(sloid.to_string())); // not enough parts
    }

    let iso = parts[0].to_lowercase();
    let uic = iso_to_uic(iso.as_str()).ok_or_else(|| OjpError::FailedToConvertIsoCode(iso))?;

    // Extract number and pad to 5 digits. Generated platform-level refs seen from the
    // live API look like `ch:1:sloid:9068_gen:ch:1:sloid:9068:0:168099_pf:3CD`; there
    // the fourth part is `9068_gen`, whose leading digits are the parent stop's SLOID
    // number, so everything from the first `_` on is dropped.
    let num_str = parts[3].split('_').next().unwrap_or(parts[3]);
    let num = num_str.parse::<i32>()?;

    let didok = format!("{}{:05}", uic, num);
    let didok = didok.parse::<i32>()?;

    Ok(didok)
}
mod duration {
    use chrono::Duration;
    use serde::Deserialize;
    use serde::de::{self, Deserializer};
    use std::str::FromStr;

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Duration, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;

        let (sign, s) = if let Some(s) = s.strip_prefix("PT") {
            (1, s)
        } else if let Some(s) = s.strip_prefix("-PT") {
            (-1, s)
        } else {
            return Err(de::Error::custom(format!(
                "duration does not start with PT or -PT, but is {s}"
            )));
        };
        // TODO: Currently -PT is treated as negative Duration. But I'm not sure what that means...

        let mut total_seconds = 0;
        let mut current_number_str = String::new();

        for c in s.chars() {
            if c.is_ascii_digit() {
                current_number_str.push(c);
            } else {
                if current_number_str.is_empty() {
                    return Err(de::Error::custom(format!(
                        "Expected a number before unit '{}'",
                        c
                    )));
                }
                let value = i64::from_str(&current_number_str).map_err(de::Error::custom)?;
                match c {
                    'H' => total_seconds += sign * value * 3600,
                    'M' => total_seconds += sign * value * 60,
                    'S' => total_seconds += sign * value,
                    _ => return Err(de::Error::custom(format!("Invalid duration unit: {}", c))),
                }
                current_number_str.clear();
            }
        }

        if !current_number_str.is_empty() {
            return Err(de::Error::custom(
                "Duration string ends with a number but no unit",
            ));
        }

        Ok(Duration::seconds(total_seconds))
    }

    // src/serde_duration.rs  (or wherever your deserialize() lives)

    #[cfg(test)]
    mod tests {
        use super::*; // brings `deserialize` into scope
        use chrono::Duration;
        use quick_xml::DeError;
        use serde::de::IntoDeserializer;

        fn parse(s: &str) -> Result<Duration, DeError> {
            let de = s.into_deserializer();
            deserialize(de)
        }

        #[test]
        fn parses_minutes_only() {
            let d = parse("PT20M").unwrap();
            assert_eq!(d, Duration::seconds(20 * 60));
        }

        #[test]
        fn parses_hours_and_minutes() {
            let d = parse("PT5H30M").unwrap();
            assert_eq!(d, Duration::seconds(5 * 3600 + 30 * 60));
        }

        #[test]
        fn parses_hours_and_minutes_and_seconds() {
            let d = parse("PT5H30M05S").unwrap();
            assert_eq!(d, Duration::seconds(5 * 3600 + 30 * 60 + 5));
        }

        #[test]
        fn parses_seconds_with_fraction() {
            let d = parse("PT1M30S").unwrap();
            let expected = Duration::seconds(60 + 30);
            assert_eq!(d, expected);
        }

        #[test]
        fn zero_duration_is_valid() {
            let d = parse("PT0S").unwrap();
            assert_eq!(d, Duration::seconds(0));
        }

        #[test]
        #[should_panic]
        fn rejects_invalid_lexical_form() {
            let _ = parse("P1D2H").unwrap();
        }

        #[test]
        fn negative_duration_accepted() {
            let d = parse("-PT10S").unwrap();
            assert_eq!(d, Duration::seconds(-10));
        }
    }
}

#[derive(Debug, Error)]
pub enum OjpError {
    #[error("Failed to parse XML {0}")]
    FailedToParseXml(DeError, String),
    #[error("Failed to find trip from id: {dep_id}, to id: {arr_id}. Optional message: {msg}")]
    FailedToFindTrip {
        dep_id: i32,
        arr_id: i32,
        msg: String,
    },
    #[error("Unkown LegType")]
    UnkownLegType,
    #[error("Failed to parse {0}")]
    ParseInt(#[from] ParseIntError),
    #[error("Failed to convert to Simplified trip")]
    FailedToConvertToSimplifiedTrip,
    #[error("Failed to get API token: {0}")]
    UnableToGetApiToken(#[from] VarError),
    #[error("Request building error: {0}")]
    RequestBuilderError(#[from] RequestError),
    #[error("No place results found")]
    PlaceResultsNotFound,
    #[error("Malformed sloid: {0}")]
    MalformedSloid(String),
    #[error("Failed to convert ISO code to UIC: {0}")]
    FailedToConvertIsoCode(String),
}

#[derive(Deserialize, Debug)]
pub struct OJP {
    #[serde(rename = "OJPResponse")]
    ojp_response: OJPResponse,
}

impl OJP {
    pub async fn find_location(
        location: &str,
        date_time: NaiveDateTime,
        number_results: u32,
        requestor_ref: &str,
        api_key: &str,
    ) -> Result<Vec<i32>, OjpError> {
        let response = RequestBuilder::try_new(date_time)?
            .set_token(token(api_key)?)
            .set_name(location)
            .set_number_results(number_results)
            .set_request_type(RequestType::LocationInformation)
            .set_requestor_ref(requestor_ref)
            .send_request()
            .await?;

        let ojp = OJP::try_from(response.as_str())?;
        let place_result = ojp
            .place_results()
            .ok_or(OjpError::PlaceResultsNotFound)?
            .into_iter()
            .filter_map(|pr| pr.stop_place_ref())
            .collect::<Vec<_>>();
        Ok::<Vec<i32>, OjpError>(place_result)
    }
    /// Given an array of `&str` containing names of places, returns  Finds `number_results` trip `from_id` to `to_id` at `date_time` using the OJP API.
    /// The name of the environment variable needs to be profived through the varibale `api_key`.
    pub async fn find_locations(
        locations: &[&str],
        date_time: NaiveDateTime,
        number_results: u32,
        requestor_ref: &str,
        api_key: &str,
    ) -> Result<Vec<i32>, OjpError> {
        let point_ref = locations
            .iter()
            .map(|&tc| async move {
                Self::find_location(tc, date_time, number_results, requestor_ref, api_key).await
            })
            .collect::<Vec<_>>();
        join_all(point_ref)
            .await
            .into_iter()
            .collect::<Result<_, _>>()
            .map(|v: Vec<_>| v.into_iter().flatten().collect())
    }

    /// Finds `number_results` trips from a list of departures and arrivals at `date_time` using the OJP API.
    /// The length of `departures` and `arrivals` must be the same; if they differ, only pairs up
    /// to the shorter length are queried and a warning is logged.
    /// The name of the environment variable needs to be profived through the varibale `api_key`.
    pub async fn find_trips(
        departures: &[i32],
        arrivals: &[i32],
        date_time: NaiveDateTime,
        number_results: u32,
        requestor_ref: &str,
        api_key: &str,
    ) -> Vec<Result<SimplifiedTrip, OjpError>> {
        if departures.len() != arrivals.len() {
            let span = span!(Level::WARN, "find_trips length mismatch");
            let _guard = span.enter();
            tracing::warn!(
                "departures ({}) and arrivals ({}) have different lengths; only the first {} pair(s) will be queried, the rest are dropped",
                departures.len(),
                arrivals.len(),
                departures.len().min(arrivals.len())
            );
        }
        let ref_trips: Vec<_> = departures
            .iter()
            .zip(arrivals.iter())
            .map(|(&from_id, &to_id)| async move {
                Self::find_trip(
                    from_id,
                    to_id,
                    date_time,
                    number_results,
                    requestor_ref,
                    api_key,
                )
                .await
            })
            .collect();
        join_all(ref_trips).await
    }

    /// Finds all trips `from_id` to `to_id` departing after `date_time` using the OJP API.
    /// For each mode in `it_modes` (walk, bike, car, ...) an additional monomodal trip is
    /// requested, so the result mixes public-transport and individual-transport trips.
    /// The name of the environment variable needs to be profived through the varibale `api_key`.
    ///
    /// Note: the Swiss endpoint has been observed to honour only one `it_modes` entry per
    /// request (see `test_xml/testing.md`); send one request per mode if you need several.
    pub async fn find_trips_with_modes(
        from_id: i32,
        to_id: i32,
        date_time: NaiveDateTime,
        number_results: u32,
        requestor_ref: &str,
        api_key: &str,
        it_modes: &[PersonalMode],
    ) -> Result<Vec<SimplifiedTrip>, OjpError> {
        let response = RequestBuilder::try_new(date_time)?
            .set_token(token(api_key)?)
            .set_from(from_id)
            .set_to(to_id)
            .set_number_results(number_results)
            .set_request_type(RequestType::Trip)
            .set_requestor_ref(requestor_ref)
            .set_it_modes(it_modes)
            .send_request()
            .await?;

        let ojp = OJP::try_from(response.as_str())?;
        if let Some(msg) = ojp.error() {
            return Err(OjpError::FailedToFindTrip {
                dep_id: from_id,
                arr_id: to_id,
                msg: msg.to_string(),
            });
        }

        ojp.trips_departing_after(date_time)
            .ok_or(OjpError::FailedToFindTrip {
                dep_id: from_id,
                arr_id: to_id,
                msg: format!("No trip departing after {date_time} was found."),
            })?
            .into_iter()
            .map(|tr| SimplifiedTrip::try_from(tr.trip()))
            .collect()
    }

    /// Finds `number_results` trip `from_id` to `to_id` at `date_time` using the OJP API.
    /// The name of the environment variable needs to be profived through the varibale `api_key`.
    pub async fn find_trip(
        from_id: i32,
        to_id: i32,
        date_time: NaiveDateTime,
        number_results: u32,
        requestor_ref: &str,
        api_key: &str,
    ) -> Result<SimplifiedTrip, OjpError> {
        let response = RequestBuilder::try_new(date_time)?
            .set_token(token(api_key)?)
            .set_from(from_id)
            .set_to(to_id)
            .set_number_results(number_results)
            .set_request_type(RequestType::Trip)
            .set_requestor_ref(requestor_ref)
            .send_request()
            .await?;

        let ojp = OJP::try_from(response.as_str()).inspect_err(|e| {
            let span = span!(Level::WARN, "From response error");
            let _guard = span.enter();
            tracing::error!("{e}");
        })?;
        let ojp = if let Some(msg) = ojp.error() {
            Err(OjpError::FailedToFindTrip {
                dep_id: from_id,
                arr_id: to_id,
                msg: msg.to_string(),
            })
        } else {
            Ok(ojp)
        }?;

        let ref_trip =
            ojp.trip_departing_after(date_time, 0)
                .ok_or(OjpError::FailedToFindTrip {
                    dep_id: from_id,
                    arr_id: to_id,
                    msg: format!("No trip departig after {date_time} was found."),
                })?;

        SimplifiedTrip::try_from(ref_trip).inspect_err(|e| {
            let span = span!(Level::WARN, "From ref_trip error");
            let _guard = span.enter();
            tracing::error!("{e}");
        })
    }

    /// Returns all trips from the OJP response
    pub fn trips(&self) -> Option<Vec<&TripResult>> {
        Some(
            self.ojp_response
                .service_delivery
                .ojp_trip_delivery
                .as_ref()?
                .trip_results
                .iter()
                .collect(),
        )
    }

    pub fn fastest_trip(&self) -> Option<&Trip> {
        let mut trips = self.trips()?;
        trips.sort_by_key(|t| t.trip.duration);
        trips.first().map(|t| &t.trip)
    }

    // Returns references over all all PlaceResults
    pub fn place_results(&self) -> Option<Vec<&PlaceResult>> {
        Some(
            self.ojp_response
                .service_delivery
                .ojp_location_information_delivery
                .as_ref()?
                .place_results
                .iter()
                .collect(),
        )
    }

    /// Returns all trips from the OJP response that are starting after `date_time`.
    /// `date_time` is interpreted as local wall-clock time
    pub fn trips_departing_after(&self, date_time: NaiveDateTime) -> Option<Vec<&TripResult>> {
        // The threshold is truncated to the whole minute before comparing: the service
        // starts monomodal (ItModeToCover) trips exactly at the requested time, so a
        // finer-grained threshold (e.g. from `Local::now()`) would wrongly exclude
        // them; timetables work at minute granularity anyway. Trips departing up to
        // 59 s before the requested instant are therefore included.
        let date_time_utc = date_time
            .and_local_timezone(Local)
            .single()?
            .naive_utc()
            .with_second(0)?
            .with_nanosecond(0)?;
        let res = self
            .trips()?
            .into_iter()
            .filter(|&t| t.trip.start_time.naive_utc() >= date_time_utc)
            .collect::<Vec<_>>();
        if res.is_empty() { None } else { Some(res) }
    }

    pub fn fastest_trip_departing_after(&self, date_time: NaiveDateTime) -> Option<&Trip> {
        let mut trips = self.trips_departing_after(date_time)?;
        trips.sort_by_key(|t| t.trip.duration);
        trips.first().map(|t| &t.trip)
    }

    /// Returns the `index`-th Trip if existing
    pub fn trip_departing_after(&self, date_time: NaiveDateTime, index: usize) -> Option<&Trip> {
        Some(
            &self
                .trips_departing_after(date_time)?
                .get(index)
                .copied()?
                .trip,
        )
    }

    /// Returns the `index`-th Trip if existing
    pub fn trip(&self, index: usize) -> Option<&Trip> {
        Some(&self.trips()?.get(index).copied()?.trip)
    }

    // Returns the error message, if the OJP trip delivery returned an error (if no trip found for
    // example)
    pub fn error(&self) -> Option<&str> {
        Some(
            self.ojp_response
                .service_delivery
                .ojp_trip_delivery
                .as_ref()?
                .error_condition
                .as_ref()?
                .trip_problem_type
                .as_str(),
        )
    }
}

impl TryFrom<&str> for OJP {
    type Error = OjpError;
    fn try_from(value: &str) -> Result<Self, Self::Error> {
        quick_xml::de::from_str(value).map_err(|e| OjpError::FailedToParseXml(e, value.to_string()))
    }
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "PascalCase")]
struct OJPResponse {
    service_delivery: ServiceDelivery,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "PascalCase")]
struct ServiceDelivery {
    response_timestamp: DateTime<Utc>,
    producer_ref: String,
    #[serde(rename = "OJPTripDelivery")]
    ojp_trip_delivery: Option<OJPTripDelivery>,
    #[serde(rename = "OJPLocationInformationDelivery")]
    ojp_location_information_delivery: Option<OJPLocationInformationDelivery>,
    #[serde(rename = "OJPStopEventDelivery")]
    ojp_stop_event_delivery: Option<OJPStopEventDelivery>,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "PascalCase")]
struct TripResponseContext {
    places: Option<Places>,
    situations: Option<Situations>,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "PascalCase")]
struct Situations {
    #[serde(default)]
    pt_situations: Vec<PtSituation>,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "PascalCase")]
struct PtSituation {
    creation_time: DateTime<Utc>,
    participation_ref: String,
    situation_number: String,
    version: i32,
    source: Source,
    validity_period: ValidityPeriod,
    alert_cause: String,
    priority: i32,
    scope_type: String,
    language: String,
    #[serde(default)]
    publishing_actions: Vec<PublishingAction>,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "PascalCase")]
struct PublishingAction {
    // TODO: Both are present until now, but they is an error that say they are missing
    // the even when present. Impossible to know why.
    publish_at_scope: Option<PublishAtScope>,
    passenger_sinformation_action: Option<PassengerInformationAction>,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "PascalCase")]
struct PassengerInformationAction {
    #[serde(default)]
    action_ref: String,
    recorded_at_time: DateTime<Utc>,
    perspective: String,
    textual_content: TextualContent,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "PascalCase")]
struct TextualContent {
    summary_content: SummaryText,
    reason_content: ReasonText,
    duration_content: DurationText,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "PascalCase")]
struct SummaryText {
    summary_text: String,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "PascalCase")]
struct ReasonText {
    reason_text: String,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "PascalCase")]
struct DurationText {
    duration_text: String,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "PascalCase")]
struct PublishAtScope {
    scope_type: String,
    #[serde(default)]
    affects: String,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "PascalCase")]
struct ValidityPeriod {
    start_time: DateTime<Utc>,
    end_time: DateTime<Utc>,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "PascalCase")]
struct Source {
    #[serde(default)]
    source_type: String,
}

#[derive(Deserialize, Debug)]
struct Places {
    #[serde(rename = "Place", default)]
    places: Vec<Place>,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "PascalCase")]
struct OJPTripDelivery {
    trip_response_context: Option<TripResponseContext>,
    #[serde(rename = "TripResult", default)]
    trip_results: Vec<TripResult>,
    error_condition: Option<ErrorCondition>,
    response_timestamp: DateTime<Utc>,
    request_message_ref: String,
    default_language: String,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "PascalCase")]
struct ErrorCondition {
    trip_problem_type: String,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "PascalCase")]
pub struct TripResult {
    id: String,
    trip: Trip,
}

impl TripResult {
    pub fn trip(&self) -> &Trip {
        &self.trip
    }
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "PascalCase")]
pub struct Trip {
    id: String,
    #[serde(with = "duration")]
    duration: Duration,
    start_time: DateTime<Utc>,
    end_time: DateTime<Utc>,
    transfers: u32,
    distance: Option<u32>,
    #[serde(rename = "Leg", default)]
    legs: Vec<Leg>,
}

impl Trip {
    pub fn legs(&self) -> Vec<&Leg> {
        self.legs.iter().collect()
    }

    pub fn departure_time(&self) -> NaiveDateTime {
        self.start_time.naive_utc()
    }

    pub fn arrival_time(&self) -> NaiveDateTime {
        self.end_time.naive_utc()
    }

    pub fn duration(&self) -> TimeDelta {
        self.duration
    }

    pub fn trip_info(&self) -> TripInfo {
        TripInfo {
            departure_time: self.start_time.naive_utc(),
            arrival_time: self.end_time.naive_utc(),
            duration: self.duration,
        }
    }
}

/// Basic trip information: departure time, arrival time, and duration
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TripInfo {
    departure_time: NaiveDateTime,
    arrival_time: NaiveDateTime,
    duration: Duration,
}

#[derive(Debug, Clone)]
pub struct SimplifiedLeg {
    departure_id: i32,
    departure_stop: String,
    arrival_id: i32,
    arrival_stop: String,
    departure_time: NaiveDateTime,
    arrival_time: NaiveDateTime,
    mode: String,
}

impl SimplifiedLeg {
    pub fn new(
        departure_id: i32,
        departure_stop: &str,
        arrival_id: i32,
        arrival_stop: &str,
        departure_time: NaiveDateTime,
        arrival_time: NaiveDateTime,
        mode: String,
    ) -> Self {
        SimplifiedLeg {
            departure_id,
            departure_stop: departure_stop.to_string(),
            arrival_id,
            arrival_stop: arrival_stop.to_string(),
            departure_time,
            arrival_time,
            mode,
        }
    }

    /// The leg's transport mode. For individual-transport legs this is a
    /// `PersonalMode` value (e.g. `"bicycle"`, parseable via
    /// `str::parse::<PersonalMode>()`); for public-transport legs it is the mode name
    /// from the timetable (e.g. `"Zug"`), and for transfer legs the transfer type
    /// (e.g. `"walk"`).
    pub fn mode(&self) -> &str {
        self.mode.as_str()
    }
}

#[derive(Debug, Clone)]
pub struct SimplifiedTrip {
    legs: Vec<SimplifiedLeg>,
}

fn utc_naive_to_local(utc_naive: NaiveDateTime) -> NaiveDateTime {
    utc_naive.and_utc().with_timezone(&Local).naive_local()
}

impl Display for SimplifiedTrip {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(
            f,
            "Trip from: {} to: {} departing at: {}",
            self.departure_stop().map_err(|_| std::fmt::Error)?,
            self.arrival_stop().map_err(|_| std::fmt::Error)?,
            utc_naive_to_local(self.departure_time().map_err(|_| std::fmt::Error)?)
        )?;
        self.legs().iter().try_for_each(|l| {
            writeln!(
                f,
                "[{:<8}]: {:<40} -> {:<40}, {} - {}",
                l.mode,
                l.departure_stop,
                l.arrival_stop,
                utc_naive_to_local(l.departure_time).format("%H:%M"),
                utc_naive_to_local(l.arrival_time).format("%H:%M")
            )
        })
    }
}

impl SimplifiedTrip {
    /// Fails if `legs` is empty: a `SimplifiedTrip` with no legs has no well-defined departure
    /// or arrival, and every accessor below relies on at least one leg being present.
    pub fn try_new(legs: Vec<SimplifiedLeg>) -> Result<Self, OjpError> {
        if legs.is_empty() {
            return Err(OjpError::FailedToConvertToSimplifiedTrip);
        }
        Ok(SimplifiedTrip { legs })
    }

    pub fn legs(&self) -> Vec<&SimplifiedLeg> {
        self.legs.iter().collect()
    }

    pub fn departure_time(&self) -> Result<NaiveDateTime, OjpError> {
        self.legs()
            .first()
            .map(|l| l.departure_time)
            .ok_or(OjpError::FailedToConvertToSimplifiedTrip)
    }

    pub fn arrival_time(&self) -> Result<NaiveDateTime, OjpError> {
        self.legs()
            .last()
            .map(|l| l.arrival_time)
            .ok_or(OjpError::FailedToConvertToSimplifiedTrip)
    }

    pub fn duration(&self) -> Result<TimeDelta, OjpError> {
        Ok(self.arrival_time()? - self.departure_time()?)
    }

    pub fn departure_id(&self) -> Result<i32, OjpError> {
        self.legs()
            .first()
            .map(|l| l.departure_id)
            .ok_or(OjpError::FailedToConvertToSimplifiedTrip)
    }

    pub fn arrival_id(&self) -> Result<i32, OjpError> {
        self.legs()
            .last()
            .map(|l| l.arrival_id)
            .ok_or(OjpError::FailedToConvertToSimplifiedTrip)
    }

    pub fn departure_stop(&self) -> Result<&str, OjpError> {
        self.legs()
            .first()
            .map(|l| l.departure_stop.as_str())
            .ok_or(OjpError::FailedToConvertToSimplifiedTrip)
    }

    pub fn arrival_stop(&self) -> Result<&str, OjpError> {
        self.legs()
            .last()
            .map(|l| l.arrival_stop.as_str())
            .ok_or(OjpError::FailedToConvertToSimplifiedTrip)
    }

    pub fn approx_equal(&self, rhs: &SimplifiedTrip, tolerance: f64) -> bool {
        let (Ok(self_departure_id), Ok(self_arrival_id), Ok(rhs_departure_id), Ok(rhs_arrival_id)) = (
            self.departure_id(),
            self.arrival_id(),
            rhs.departure_id(),
            rhs.arrival_id(),
        ) else {
            return false;
        };
        // deprature and arrival must be the same
        if self_departure_id != rhs_departure_id || self_arrival_id != rhs_arrival_id {
            return false;
        }

        let (Ok(self_duration), Ok(rhs_duration)) = (self.duration(), rhs.duration()) else {
            return false;
        };
        // duration must be approximately equal
        if (self_duration.as_seconds_f64() - rhs_duration.as_seconds_f64()).abs()
            / rhs_duration.as_seconds_f64()
            > tolerance
        {
            return false;
        }

        let (
            Ok(self_departure_time),
            Ok(self_arrival_time),
            Ok(rhs_departure_time),
            Ok(rhs_arrival_time),
        ) = (
            self.departure_time(),
            self.arrival_time(),
            rhs.departure_time(),
            rhs.arrival_time(),
        )
        else {
            return false;
        };
        // departure and arrival time must be approximately the same with respect to duration
        if ((self_departure_time - rhs_departure_time).as_seconds_f64()
            / self_duration.as_seconds_f64())
        .abs()
            > tolerance
            || ((self_arrival_time - rhs_arrival_time).as_seconds_f64()
                / self_duration.as_seconds_f64())
            .abs()
                > tolerance
        {
            return false;
        }
        true
    }
}

impl TryFrom<&Trip> for SimplifiedTrip {
    type Error = OjpError;
    fn try_from(value: &Trip) -> Result<Self, Self::Error> {
        let mut prev_arr_time = value.start_time.naive_utc();
        let st: Vec<_> = value
            .legs()
            .into_iter()
            .map(|leg| {
                let typed_leg = LegType::try_from(leg)?;
                let departure_id = typed_leg.departure_id()?;
                let departure_stop = typed_leg.departure_stop();
                let arrival_id = typed_leg.arrival_id()?;
                let arrival_stop = typed_leg.arrival_stop();
                let departure_time = typed_leg.departure_time().unwrap_or(prev_arr_time);
                let arrival_time = typed_leg
                    .arrival_time()
                    .unwrap_or(prev_arr_time + typed_leg.duration());
                prev_arr_time = arrival_time;
                Ok(SimplifiedLeg::new(
                    departure_id,
                    departure_stop,
                    arrival_id,
                    arrival_stop,
                    departure_time,
                    arrival_time,
                    typed_leg.mode().to_string(),
                ))
            })
            .collect::<Result<Vec<_>, OjpError>>()?;
        SimplifiedTrip::try_new(st)
    }
}

pub enum LegType<'a> {
    Timed(&'a TimedLeg),
    Transfer(&'a TransferLeg),
    Continuous(&'a ContinuousLeg),
}

impl<'a> LegType<'a> {
    pub fn duration(&'a self) -> TimeDelta {
        match *self {
            Self::Timed(tl) => tl.arrival_time() - tl.departure_time(),
            Self::Transfer(t) => t.duration,
            Self::Continuous(t) => t.duration,
        }
    }

    pub fn departure_time(&'a self) -> Option<NaiveDateTime> {
        match *self {
            Self::Timed(tl) => Some(tl.departure_time().naive_utc()),
            Self::Transfer(_) => None,
            Self::Continuous(_) => None,
        }
    }

    pub fn arrival_time(&'a self) -> Option<NaiveDateTime> {
        match *self {
            Self::Timed(tl) => Some(tl.arrival_time().naive_utc()),
            Self::Transfer(_) => None,
            Self::Continuous(_) => None,
        }
    }

    pub fn departure_stop(&'a self) -> &'a str {
        match *self {
            Self::Timed(tl) => tl.departure_stop(),
            Self::Transfer(t) => t.departure_stop(),
            Self::Continuous(t) => t.departure_stop(),
        }
    }

    pub fn arrival_stop(&'a self) -> &'a str {
        match *self {
            Self::Timed(tl) => tl.arrival_stop(),
            Self::Transfer(t) => t.arrival_stop(),
            Self::Continuous(t) => t.arrival_stop(),
        }
    }

    pub fn departure_id(&'a self) -> Result<i32, OjpError> {
        match *self {
            Self::Timed(tl) => tl.departure_id(),
            Self::Transfer(t) => t.departure_id(),
            Self::Continuous(t) => t.departure_id(),
        }
    }

    pub fn arrival_id(&'a self) -> Result<i32, OjpError> {
        match *self {
            Self::Timed(tl) => tl.arrival_id(),
            Self::Transfer(t) => t.arrival_id(),
            Self::Continuous(t) => t.arrival_id(),
        }
    }

    pub fn mode(&self) -> &str {
        match *self {
            Self::Timed(tl) => tl.service.mode.name(),
            Self::Transfer(t) => t.transfer_type.as_str(),
            Self::Continuous(t) => t.service.personal_mode.as_str(),
        }
    }
}

impl<'a> TryFrom<&'a Leg> for LegType<'a> {
    type Error = OjpError;
    fn try_from(value: &'a Leg) -> Result<Self, Self::Error> {
        if let Some(l) = value.timed_leg.as_ref() {
            Ok(LegType::Timed(l))
        } else if let Some(l) = value.transfer_leg.as_ref() {
            Ok(LegType::Transfer(l))
        } else if let Some(l) = value.continuous_leg.as_ref() {
            Ok(LegType::Continuous(l))
        } else {
            Err(OjpError::UnkownLegType)
        }
    }
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "PascalCase")]
pub struct Leg {
    id: u32,
    #[serde(with = "duration")]
    duration: Duration,
    timed_leg: Option<TimedLeg>,
    transfer_leg: Option<TransferLeg>,
    continuous_leg: Option<ContinuousLeg>,
    #[serde(rename = "EmissionCO2")]
    emission_co2: Option<EmissionCO2>,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "PascalCase")]
pub struct ContinuousLeg {
    leg_start: LegEndpoint,
    leg_end: LegEndpoint,
    service: ContinuousService,
    #[serde(with = "duration")]
    duration: Duration,
    // Monomodal trips requested via ItModeToCover come back with only the fields above
    // (see test_xml/trip_bicycle.xml), so everything below must stay optional.
    length: Option<i32>,
    leg_track: Option<LegTrack>,
    path_guidance: Option<PathGuidance>,
}

impl ContinuousLeg {
    pub fn departure_stop(&self) -> &str {
        self.leg_start.name()
    }

    pub fn arrival_stop(&self) -> &str {
        self.leg_end.name()
    }

    pub fn departure_id(&self) -> Result<i32, OjpError> {
        self.leg_start.id()
    }

    pub fn arrival_id(&self) -> Result<i32, OjpError> {
        self.leg_end.id()
    }
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "PascalCase")]
struct ContinuousService {
    personal_mode_of_operation: String,
    personal_mode: String,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "PascalCase")]
struct PathGuidance {
    #[serde(rename = "PathGuidanceSection", default)]
    path_guidance_sections: Vec<PathGuidanceSection>,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "PascalCase")]
struct PathGuidanceSection {
    track_section: TrackSection,
    turn_description: Text,
    guidance_advice: String,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "PascalCase")]
pub struct TransferLeg {
    transfer_type: String,
    leg_start: LegEndpoint,
    leg_end: LegEndpoint,
    #[serde(with = "duration")]
    duration: Duration,
}

impl TransferLeg {
    pub fn departure_stop(&self) -> &str {
        self.leg_start.name()
    }

    pub fn arrival_stop(&self) -> &str {
        self.leg_end.name()
    }

    pub fn departure_id(&self) -> Result<i32, OjpError> {
        self.leg_start.id()
    }

    pub fn arrival_id(&self) -> Result<i32, OjpError> {
        self.leg_end.id()
    }
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "PascalCase")]
struct LegEndpoint {
    stop_point_ref: String,
    name: Text,
}

impl LegEndpoint {
    pub fn id(&self) -> Result<i32, OjpError> {
        if let Ok(num) = self.stop_point_ref.parse::<i32>() {
            Ok(num)
        } else {
            sloid_to_didok(&self.stop_point_ref)
        }
    }
    pub fn name(&self) -> &str {
        self.name.text.as_str()
    }
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "PascalCase")]
pub struct TimedLeg {
    leg_board: LegBoard,
    leg_alight: LegAlight,
    #[serde(rename = "LegIntermediate", default)]
    leg_intermediates: Vec<LegIntermediate>,
    service: Service,
    leg_track: Option<LegTrack>,
}

impl TimedLeg {
    pub fn departure_time(&self) -> DateTime<Utc> {
        if let Some(time) = self.leg_board.service_departure.estimated_time {
            time
        } else {
            self.leg_board.service_departure.timetabled_time
        }
    }

    pub fn arrival_time(&self) -> DateTime<Utc> {
        if let Some(time) = self.leg_alight.service_arrival.estimated_time {
            time
        } else {
            self.leg_alight.service_arrival.timetabled_time
        }
    }

    pub fn departure_id(&self) -> Result<i32, OjpError> {
        self.leg_board.id()
    }

    pub fn arrival_id(&self) -> Result<i32, OjpError> {
        self.leg_alight.id()
    }

    pub fn departure_stop(&self) -> &str {
        self.leg_board.name()
    }

    pub fn arrival_stop(&self) -> &str {
        self.leg_alight.name()
    }
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "PascalCase")]
struct LegIntermediate {
    stop_point_ref: String,
    stop_point_name: Text,
    name_suffix: Option<Text>,
    planned_quay: Option<Text>,
    service_arrival: Option<ServiceArrival>,
    service_departure: Option<ServiceDeparture>,
    order: u32,
    #[serde(rename = "ExpectedDepartureOccupancy", default)]
    expected_departure_occupancies: Vec<ExpectedDepartureOccupancy>,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "PascalCase")]
struct LegBoard {
    stop_point_ref: String,
    stop_point_name: Text,
    name_suffix: Option<Text>,
    planned_quay: Option<Text>,
    estimated_quay: Option<Text>,
    service_departure: ServiceDeparture,
    order: u32,
    #[serde(rename = "ExpectedDepartureOccupancy", default)]
    expected_departure_occupancies: Vec<ExpectedDepartureOccupancy>,
}

impl LegBoard {
    pub fn id(&self) -> Result<i32, OjpError> {
        if let Ok(num) = self.stop_point_ref.parse::<i32>() {
            Ok(num)
        } else {
            sloid_to_didok(&self.stop_point_ref)
        }
    }
    pub fn name(&self) -> &str {
        self.stop_point_name.text.as_str()
    }
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "PascalCase")]
struct LegAlight {
    stop_point_ref: String,
    stop_point_name: Text,
    name_suffix: Option<Text>,
    planned_quay: Option<Text>,
    estimated_quay: Option<Text>,
    service_arrival: ServiceArrival,
    order: u32,
}

impl LegAlight {
    pub fn id(&self) -> Result<i32, OjpError> {
        if let Ok(num) = self.stop_point_ref.parse::<i32>() {
            Ok(num)
        } else {
            sloid_to_didok(&self.stop_point_ref)
        }
    }
    pub fn name(&self) -> &str {
        self.stop_point_name.text.as_str()
    }
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "PascalCase")]
struct ServiceDeparture {
    timetabled_time: DateTime<Utc>,
    estimated_time: Option<DateTime<Utc>>,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "PascalCase")]
struct ServiceArrival {
    timetabled_time: DateTime<Utc>,
    estimated_time: Option<DateTime<Utc>>,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "PascalCase")]
struct Service {
    operating_day_ref: String,
    journey_ref: String,
    public_code: String,
    line_ref: String,
    direction_ref: String,
    mode: Mode,
    product_category: Option<ProductCategory>,
    published_service_name: Text,
    train_number: String,
    #[serde(rename = "Attribute", default)]
    attributes: Vec<Attribute>,
    origin_text: Text,
    operator_ref: String,
    destination_stop_point_ref: String,
    destination_text: Text,
    #[serde(default)]
    origin_stop_point_ref: String,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "PascalCase")]
struct Mode {
    pt_mode: String,
    #[serde(default)]
    rail_submode: String,
    name: Text,
    short_name: Text,
}

impl Mode {
    pub fn name(&self) -> &str {
        self.name.text.as_str()
    }
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "PascalCase")]
struct ProductCategory {
    name: Text,
    short_name: Text,
    product_category_ref: String,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "PascalCase")]
struct Attribute {
    user_text: Text,
    code: String,
    importance: u32,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "PascalCase")]
struct Text {
    text: String,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "PascalCase")]
struct EmissionCO2 {
    kilogram_per_person_km: f32,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "PascalCase")]
struct ExpectedDepartureOccupancy {
    fare_class: String,
    occupancy_level: String,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "PascalCase")]
struct LegTrack {
    // maxOccurs="unbounded" in the OJP 2.0 schema (LegTrackStructure).
    #[serde(rename = "TrackSection", default)]
    track_sections: Vec<TrackSection>,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "PascalCase")]
struct TrackSection {
    track_section_start: Option<TrackSectionEndpoint>,
    track_section_end: Option<TrackSectionEndpoint>,
    link_projection: Option<LinkProjection>,
    road_name: Option<String>,
    #[serde(with = "duration")]
    duration: Duration,
    length: i32,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "PascalCase")]
struct TrackSectionEndpoint {
    stop_point_ref: String,
    name: Text,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "PascalCase")]
struct LinkProjection {
    #[serde(rename = "Position", default)]
    positions: Vec<Position>,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "PascalCase")]
struct Position {
    longitude: f64,
    latitude: f64,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "PascalCase")]
struct OJPLocationInformationDelivery {
    #[serde(rename = "PlaceResult", default)]
    place_results: Vec<PlaceResult>,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "PascalCase")]
struct StopEventResponseContext {
    places: Places,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "PascalCase")]
struct OJPStopEventDelivery {
    stop_event_response_context: Option<StopEventResponseContext>,
    #[serde(rename = "StopEventResult", default)]
    stop_event_results: Vec<StopEventResult>,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "PascalCase")]
struct StopEventResult {
    id: String,
    stop_event: StopEvent,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "PascalCase")]
struct StopEvent {
    this_call: ThisCall,
    service: Service,
    operating_days: Option<OperatingDays>,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "PascalCase")]
struct OperatingDays {
    from: String,
    to: String,
    pattern: String,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "PascalCase")]
struct ThisCall {
    call_at_stop: CallAtStop,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "PascalCase")]
struct CallAtStop {
    stop_point_ref: String,
    stop_point_name: Text,
    service_departure: Option<ServiceDeparture>,
    service_arrival: Option<ServiceArrival>,
    order: u32,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "PascalCase")]
pub struct PlaceResult {
    place: Place,
    complete: bool,
    probability: f64,
}

impl PlaceResult {
    pub fn stop_place_ref(&self) -> Option<i32> {
        self.place.stop_place.as_ref()?.id().ok()
    }

    pub fn stop_place_name(&self) -> Option<&str> {
        Some(&self.place.stop_place.as_ref()?.stop_place_name.text)
    }
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "PascalCase")]
struct Place {
    stop_place: Option<StopPlace>,
    topographic_place: Option<TopographicPlace>,
    stop_point: Option<StopPoint>,
    name: Text,
    geo_position: GeoPosition,
    #[serde(rename = "Mode", default)]
    place_modes: Vec<PlaceMode>,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "PascalCase")]
struct StopPoint {
    stop_point_ref: String,
    stop_point_name: Text,
    private_code: PrivateCode,
    parent_ref: String,
    topographic_place_ref: String,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "PascalCase")]
struct TopographicPlace {
    topographic_place_code: String,
    topographic_place_name: Text,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "PascalCase")]
struct StopPlace {
    stop_place_ref: String,
    stop_place_name: Text,
    private_code: PrivateCode,
    topographic_place_ref: String,
}

impl StopPlace {
    pub fn id(&self) -> Result<i32, OjpError> {
        if let Ok(num) = self.stop_place_ref.parse::<i32>() {
            Ok(num)
        } else {
            sloid_to_didok(&self.stop_place_ref)
        }
    }
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "PascalCase")]
struct PrivateCode {
    system: String,
    value: String,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "PascalCase")]
struct GeoPosition {
    longitude: f64,
    latitude: f64,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "PascalCase")]
struct PlaceMode {
    pt_mode: String,
    #[serde(default)]
    rail_submode: String,
    #[serde(default)]
    tram_submode: String,
    #[serde(default)]
    bus_submode: String,
    #[serde(default)]
    funicular_submode: String,
}

#[cfg(test)]
mod test {
    use crate::{
        OJP, OjpError, PersonalMode, RequestBuilder, RequestType, SimplifiedLeg, SimplifiedTrip,
        token,
    };
    use chrono::{Duration, Local, NaiveDate, NaiveDateTime, NaiveTime};
    use std::error::Error;
    use test_log::test;

    const FORMAT: &str = "%Y-%m-%dT%H:%M:%SZ";

    #[allow(unused)]
    fn parse_xml(xml: &str) -> Result<OJP, Box<dyn Error>> {
        let xml = std::fs::read_to_string(xml)?;
        let ojp = super::OJP::try_from(xml.as_str())?;
        Ok(ojp)
    }

    #[test]
    fn sloid_to_didok_converts_known_examples() {
        // Real StopPlaceRef seen from the live API for "Bern Bümpliz Süd".
        assert_eq!(super::sloid_to_didok("ch:1:sloid:4106").unwrap(), 8504106);
        assert_eq!(super::sloid_to_didok("de:1:sloid:42").unwrap(), 8000042);
        // Generated platform-level ref (see test_xml/trip_generated_sloid.xml): the
        // digits before `_gen` are the parent stop's SLOID number.
        assert_eq!(
            super::sloid_to_didok("ch:1:sloid:9068_gen:ch:1:sloid:9068:0:168099_pf:3CD").unwrap(),
            8509068
        );
        assert!(super::sloid_to_didok("not-a-sloid").is_err());
        assert!(super::sloid_to_didok("ch:1:sloid:_gen").is_err());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn find_trips_truncates_on_length_mismatch() {
        let date_time = NaiveDateTime::new(
            NaiveDate::from_ymd_opt(2025, 6, 1).unwrap(),
            NaiveTime::from_hms_opt(10, 0, 0).unwrap(),
        );
        // Mismatched lengths: 2 departures, 1 arrival. A deliberately nonexistent env var name is
        // used for `api_key` so this never makes a real network call (token lookup fails first) --
        // we only care that the returned Vec's length matches the shorter input rather than
        // silently including a spurious element or panicking.
        let results = OJP::find_trips(
            &[1, 2],
            &[10],
            date_time,
            1,
            "Test",
            "OJP_RS_TEST_NONEXISTENT_TOKEN_VAR",
        )
        .await;
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn simplified_trip_try_new_rejects_empty_legs() {
        assert!(matches!(
            SimplifiedTrip::try_new(vec![]),
            Err(OjpError::FailedToConvertToSimplifiedTrip)
        ));
    }

    #[test]
    fn simplified_trip_accessors_error_on_empty_legs() {
        // Bypasses `try_new` (possible here since the test module is nested inside `model` and
        // can see private fields) to confirm the accessors themselves stay defensive -- returning
        // an error rather than panicking -- even if the non-empty invariant is ever violated by a
        // future construction path.
        let empty = super::SimplifiedTrip { legs: vec![] };
        assert!(empty.departure_time().is_err());
        assert!(empty.arrival_time().is_err());
        assert!(empty.departure_id().is_err());
        assert!(empty.arrival_id().is_err());
        assert!(empty.departure_stop().is_err());
        assert!(empty.arrival_stop().is_err());
        assert!(empty.duration().is_err());
        assert!(!empty.approx_equal(&empty, 0.01));
    }

    #[test]
    fn display_shows_local_time_not_utc() {
        // SimplifiedLeg/SimplifiedTrip store naive UTC times; Display is the human-facing
        // summary (used e.g. by examples/find_journeys.rs) and must show local wall-clock time.
        let departure_utc = NaiveDateTime::new(
            NaiveDate::from_ymd_opt(2025, 7, 15).unwrap(),
            NaiveTime::from_hms_opt(8, 0, 0).unwrap(),
        );
        let arrival_utc = departure_utc + Duration::hours(1);
        let trip = SimplifiedTrip::try_new(vec![SimplifiedLeg::new(
            1,
            "A",
            2,
            "B",
            departure_utc,
            arrival_utc,
            "rail".to_string(),
        )])
        .unwrap();

        let expected_departure_local = super::utc_naive_to_local(departure_utc);
        let expected_arrival_local = super::utc_naive_to_local(arrival_utc);
        let printed = trip.to_string();

        assert!(printed.contains(&expected_departure_local.format("%H:%M").to_string()));
        assert!(printed.contains(&expected_arrival_local.format("%H:%M").to_string()));
    }

    fn one_hour_trip_departing_at(departure: NaiveDateTime) -> SimplifiedTrip {
        SimplifiedTrip::try_new(vec![SimplifiedLeg::new(
            1,
            "A",
            2,
            "B",
            departure,
            departure + Duration::hours(1),
            "rail".to_string(),
        )])
        .unwrap()
    }

    #[test]
    fn approx_equal_detects_time_mismatch_regardless_of_direction() {
        let base_departure = NaiveDateTime::new(
            NaiveDate::from_ymd_opt(2025, 6, 1).unwrap(),
            NaiveTime::from_hms_opt(10, 0, 0).unwrap(),
        );
        let earlier = one_hour_trip_departing_at(base_departure);
        let later = one_hour_trip_departing_at(base_departure + Duration::seconds(1000));

        // `earlier` departs well before `later` (>1% of the 1h duration apart); this must not
        // be considered approximately equal regardless of comparison order.
        assert!(!earlier.approx_equal(&later, 0.01));
        assert!(!later.approx_equal(&earlier, 0.01));
    }

    #[test]
    fn approx_equal_accepts_trips_within_tolerance() {
        let base_departure = NaiveDateTime::new(
            NaiveDate::from_ymd_opt(2025, 6, 1).unwrap(),
            NaiveTime::from_hms_opt(10, 0, 0).unwrap(),
        );
        let on_time = one_hour_trip_departing_at(base_departure);
        // 10s off a 1h (3600s) trip is well within a 1% tolerance (36s).
        let slightly_late = one_hour_trip_departing_at(base_departure + Duration::seconds(10));

        assert!(on_time.approx_equal(&slightly_late, 0.01));
        assert!(slightly_late.approx_equal(&on_time, 0.01));
    }

    #[test]
    fn location_coordinate() {
        let _ojp = parse_xml("test_xml/location_coordinate.xml").unwrap();
    }

    #[test]
    fn location_extended() {
        let _ojp = parse_xml("test_xml/location_extended.xml").unwrap();
    }

    #[test]
    fn location_simple() {
        let ojp = parse_xml("test_xml/location_simple.xml").unwrap();
        let place_results = ojp.place_results().unwrap();
        assert_eq!(place_results.len(), 14);
    }

    #[test]
    fn location_topographic() {
        let _ojp = parse_xml("test_xml/location_topographic.xml").unwrap();
    }

    #[test]
    fn stop_simple() {
        let _ojp = parse_xml("test_xml/stop_simple.xml").unwrap();
    }

    #[test]
    fn stop_complex() {
        let _ojp = parse_xml("test_xml/stop_complex.xml").unwrap();
    }

    #[test]
    fn trip_simple() {
        let ojp = parse_xml("test_xml/trip_simple.xml").unwrap();
        let fastest_trip = ojp.fastest_trip().unwrap();
        assert_eq!(fastest_trip.duration.num_seconds(), 3 * 60 + 30);

        // `trip_departing_after`/`trips_departing_after` interpret their `date_time` argument as
        // local wall-clock time; convert the UTC threshold we care about to its local-time
        // equivalent so the test is correct regardless of the host's timezone.
        let threshold_utc = NaiveDateTime::parse_from_str("2025-10-17T09:00:00Z", FORMAT).unwrap();
        let threshold_local = threshold_utc.and_utc().with_timezone(&Local).naive_local();

        let trip_after = ojp.trip_departing_after(threshold_local, 0).unwrap();

        assert_eq!(
            trip_after.start_time.naive_utc(),
            NaiveDateTime::parse_from_str("2025-10-17T09:07:24Z", FORMAT).unwrap()
        );
        assert_eq!(
            trip_after.end_time.naive_utc(),
            NaiveDateTime::parse_from_str("2025-10-17T09:10:54Z", FORMAT).unwrap()
        );

        assert_eq!(trip_after.id, "ID-5CE0364E-BD0F-4D17-929E-B3E4F4EAA714");

        let simplified_trip = SimplifiedTrip::try_from(trip_after).unwrap();
        assert_eq!(
            simplified_trip.departure_time().unwrap(),
            NaiveDateTime::parse_from_str("2025-10-17T09:07:24Z", FORMAT).unwrap()
        );
        assert_eq!(
            simplified_trip.arrival_time().unwrap(),
            NaiveDateTime::parse_from_str("2025-10-17T09:10:54Z", FORMAT).unwrap()
        );

        let trips = ojp.trips().unwrap();
        assert_eq!(trips.len(), 3);
        assert_eq!(ojp.trips_departing_after(threshold_local).unwrap().len(), 2);
    }

    #[test]
    fn trip_lots() {
        let _ojp = parse_xml("test_xml/trip_lots.xml").unwrap();
    }

    /// Parses an ItModeToCover fixture (see `test_xml/testing.md`), asserts it holds
    /// `expected_trips` trip results that all convert to `SimplifiedTrip`, and returns
    /// the single-leg monomodal trip with `mode`.
    fn monomodal_trip_from_fixture(
        fixture: &str,
        mode: PersonalMode,
        expected_trips: usize,
    ) -> SimplifiedTrip {
        let ojp = parse_xml(fixture).unwrap();
        let trips = ojp.trips().unwrap();
        assert_eq!(trips.len(), expected_trips);

        let simplified: Vec<_> = trips
            .iter()
            .map(|t| SimplifiedTrip::try_from(t.trip()).unwrap())
            .collect();
        let monomodal = simplified
            .into_iter()
            .find(|st| st.legs()[0].mode == mode.as_str())
            .unwrap_or_else(|| panic!("no monomodal {mode:?} trip in {fixture}"));
        assert_eq!(monomodal.legs().len(), 1);
        monomodal
    }

    #[test]
    fn trip_foot() {
        // Short distance (Zürich HB -> Zürich, Sihlquai/HB, 413 m; the endpoint only
        // returns walk trips below a distance limit). Unlike the other monomodal
        // fixtures, the walk ContinuousLeg carries a LegTrack and a PathGuidance with
        // per-section TrackSections, pinning the richer variant of the structure.
        let walk = monomodal_trip_from_fixture("test_xml/trip_foot.xml", PersonalMode::Foot, 2);
        assert_eq!(walk.departure_id().unwrap(), 8503000);
        assert_eq!(walk.arrival_id().unwrap(), 8591368);
        assert_eq!(walk.duration().unwrap(), Duration::seconds(6 * 60 + 16));
    }

    #[test]
    fn trips_departing_after_truncates_to_the_minute() {
        // Regression test: monomodal trips start exactly at the requested time (with
        // at most millisecond precision, as echoed by the server), so a finer-grained
        // threshold as produced by `Local::now()` must not exclude them. It used to:
        // the bicycle trip starting at 08:00:00.000 lost against a threshold of
        // 08:00:00.000000500 by less than a microsecond.
        let ojp = parse_xml("test_xml/trip_bicycle.xml").unwrap();
        let start_utc = NaiveDateTime::parse_from_str("2026-07-03T08:00:00Z", FORMAT).unwrap();
        let threshold_local = start_utc.and_utc().with_timezone(&Local).naive_local()
            + Duration::seconds(30)
            + Duration::nanoseconds(500);

        // The threshold 08:00:30.000000500 is truncated to 08:00:00, so all 5 trips
        // survive the filter (4 public-transport ones departing later, plus the
        // bicycle trip starting exactly at 08:00:00).
        assert_eq!(ojp.trips_departing_after(threshold_local).unwrap().len(), 5);
    }

    #[test]
    fn trip_bicycle() {
        // 4 public-transport trips plus one monomodal bicycle trip consisting of a
        // single ContinuousLeg (no LegTrack/PathGuidance, no own timestamps).
        let bike =
            monomodal_trip_from_fixture("test_xml/trip_bicycle.xml", PersonalMode::Bicycle, 5);
        // SLOID stop refs (ch:1:sloid:3000/3006) must convert to DIDOK ids.
        assert_eq!(bike.departure_id().unwrap(), 8503000);
        assert_eq!(bike.arrival_id().unwrap(), 8503006);
        assert_eq!(bike.departure_stop().unwrap(), "Zürich HB");
        assert_eq!(bike.arrival_stop().unwrap(), "Zürich Oerlikon");
        // The ContinuousLeg has no timestamps of its own; they are derived from the
        // trip's start time plus the leg duration (PT21M17S).
        assert_eq!(
            bike.departure_time().unwrap(),
            NaiveDateTime::parse_from_str("2026-07-03T08:00:00Z", FORMAT).unwrap()
        );
        assert_eq!(bike.duration().unwrap(), Duration::seconds(21 * 60 + 17));
    }

    #[test]
    fn trip_car() {
        let car = monomodal_trip_from_fixture("test_xml/trip_car.xml", PersonalMode::Car, 5);
        assert_eq!(car.departure_id().unwrap(), 8503000);
        assert_eq!(car.arrival_id().unwrap(), 8503006);
        assert_eq!(car.duration().unwrap(), Duration::seconds(7 * 60 + 44));
    }

    #[test]
    fn trip_motorcycle() {
        let motorcycle = monomodal_trip_from_fixture(
            "test_xml/trip_motorcycle.xml",
            PersonalMode::Motorcycle,
            5,
        );
        assert_eq!(motorcycle.departure_id().unwrap(), 8503000);
        assert_eq!(motorcycle.arrival_id().unwrap(), 8503006);
        assert_eq!(
            motorcycle.duration().unwrap(),
            Duration::seconds(8 * 60 + 20)
        );
    }

    #[test]
    fn trip_scooter() {
        let scooter =
            monomodal_trip_from_fixture("test_xml/trip_scooter.xml", PersonalMode::Scooter, 5);
        assert_eq!(scooter.departure_id().unwrap(), 8503000);
        assert_eq!(scooter.arrival_id().unwrap(), 8503006);
        assert_eq!(
            scooter.duration().unwrap(),
            Duration::seconds(3600 + 32 * 60 + 16)
        );
    }

    #[test]
    fn trip_truck() {
        // The endpoint silently ignores `truck` (see test_xml/testing.md): the response
        // is valid but contains only the public-transport trips. Pins both that such a
        // response still parses and the ignore behaviour at capture time.
        let ojp = parse_xml("test_xml/trip_truck.xml").unwrap();
        let trips = ojp.trips().unwrap();
        assert_eq!(trips.len(), 4);

        let simplified: Vec<_> = trips
            .iter()
            .map(|t| SimplifiedTrip::try_from(t.trip()).unwrap())
            .collect();
        assert!(simplified.iter().all(|st| {
            st.legs()
                .iter()
                .all(|l| l.mode != PersonalMode::Truck.as_str())
        }));
    }

    #[test]
    fn trip_generated_sloid() {
        // Regression test: this response (Le Landeron 8500994 → Alle, Grands Prés
        // 8574834) contains generated platform-level refs like
        // `ch:1:sloid:9068_gen:ch:1:sloid:9068:0:168099_pf:3CD`, which used to fail
        // SimplifiedTrip conversion with "invalid digit found in string".
        let ojp = parse_xml("test_xml/trip_generated_sloid.xml").unwrap();
        let trips = ojp.trips().unwrap();
        assert_eq!(trips.len(), 5);
        for t in trips {
            SimplifiedTrip::try_from(t.trip()).unwrap();
        }
    }

    #[tokio::test(flavor = "current_thread")]
    #[test_log::test]
    async fn request_location_information_service_simple() {
        dotenvy::dotenv().ok(); // optional
        let date_time = NaiveDateTime::new(
            NaiveDate::from_ymd_opt(2025, 11, 19).unwrap(),
            NaiveTime::from_hms_milli_opt(20, 56, 28, 643).unwrap(),
        );
        let response = RequestBuilder::try_new(date_time)
            .unwrap()
            .set_token(token("TOKEN").unwrap())
            .set_requestor_ref("Test")
            .set_name("bern s")
            .set_number_results(3)
            .set_request_type(RequestType::LocationInformation)
            .send_request()
            .await
            .unwrap();
        let _ojp = OJP::try_from(response.as_str()).unwrap();
    }

    #[tokio::test(flavor = "current_thread")]
    #[test_log::test]
    async fn request_trip_with_every_personal_mode() {
        dotenvy::dotenv().ok(); // optional
        let date_time = NaiveDateTime::new(
            NaiveDate::from_ymd_opt(2026, 7, 3).unwrap(),
            NaiveTime::from_hms_opt(10, 0, 0).unwrap(),
        );
        // One request per mode: the Swiss endpoint honours only a single ItModeToCover
        // per request (see test_xml/testing.md). Most modes use Zürich HB -> Oerlikon
        // (~4.7 km); foot must stay within the endpoint's walking-distance policy, so it
        // uses Zürich HB -> Sihlquai/HB (413 m). The bool records whether the endpoint
        // returns a monomodal trip for that mode: truck is silently ignored (plain
        // public-transport results come back), so only the round trip is verified there.
        let cases = [
            (PersonalMode::Foot, 8591368, true),
            (PersonalMode::Bicycle, 8503006, true),
            (PersonalMode::Car, 8503006, true),
            (PersonalMode::Motorcycle, 8503006, true),
            (PersonalMode::Truck, 8503006, false),
            (PersonalMode::Scooter, 8503006, true),
        ];
        for (mode, to_id, returns_monomodal_trip) in cases {
            let trips =
                OJP::find_trips_with_modes(8503000, to_id, date_time, 3, "Test", "TOKEN", &[mode])
                    .await
                    .unwrap();

            assert!(!trips.is_empty(), "no trips at all for {mode:?}");
            if returns_monomodal_trip {
                assert!(
                    trips
                        .iter()
                        .any(|t| t.legs().iter().any(|l| l.mode == mode.as_str())),
                    "expected a monomodal {mode:?} trip among the results"
                );
            }
        }
    }

    #[tokio::test(flavor = "current_thread")]
    #[test_log::test]
    async fn request_trip_service_simple() {
        dotenvy::dotenv().ok(); // optional
        let date_time = NaiveDateTime::new(
            NaiveDate::from_ymd_opt(2025, 11, 19).unwrap(),
            NaiveTime::from_hms_milli_opt(20, 56, 28, 643).unwrap(),
        );
        let response = RequestBuilder::try_new(date_time)
            .unwrap()
            .set_token(token("TOKEN").unwrap())
            .set_requestor_ref("Test")
            .set_number_results(3)
            .set_request_type(RequestType::Trip)
            .set_from(8503308)
            .set_to(8503424)
            .send_request()
            .await
            .unwrap();
        let _ojp = OJP::try_from(response.as_str()).unwrap();
    }
}
