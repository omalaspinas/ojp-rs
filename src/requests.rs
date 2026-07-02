use std::fmt::Display;
use std::str::FromStr;

use chrono::{DateTime, Local, NaiveDateTime, SecondsFormat, Utc};
use reqwest::Client;
use secrecy::{ExposeSecret, SecretString};
use thiserror::Error;
use tracing::{Level, span};

const URL: &str = "https://api.opentransportdata.swiss/ojp20";

#[derive(Debug)]
pub enum RequestType {
    LocationInformation,
    Trip,
    StopEvent,
    Unknown,
}

/// Individual ("personal") transport mode, as defined by the OJP 2.0
/// `PersonalModesEnumeration`. Used with [`RequestBuilder::set_it_modes`] to request
/// monomodal trips (e.g. a pure bicycle route) in addition to public-transport results.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PersonalMode {
    Foot,
    Bicycle,
    Car,
    Motorcycle,
    Truck,
    Scooter,
}

impl PersonalMode {
    /// The value used in the request XML (`<PersonalMode>` element).
    pub fn as_str(self) -> &'static str {
        match self {
            PersonalMode::Foot => "foot",
            PersonalMode::Bicycle => "bicycle",
            PersonalMode::Car => "car",
            PersonalMode::Motorcycle => "motorcycle",
            PersonalMode::Truck => "truck",
            PersonalMode::Scooter => "scooter",
        }
    }
}

impl Display for PersonalMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for PersonalMode {
    type Err = RequestError;

    /// Parses an OJP 2.0 `PersonalModesEnumeration` value — the same strings produced
    /// by [`PersonalMode::as_str`] and found in response elements such as a
    /// `ContinuousLeg`'s `<PersonalMode>` (surfaced as `SimplifiedLeg`'s `mode`).
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "foot" => Ok(PersonalMode::Foot),
            "bicycle" => Ok(PersonalMode::Bicycle),
            "car" => Ok(PersonalMode::Car),
            "motorcycle" => Ok(PersonalMode::Motorcycle),
            "truck" => Ok(PersonalMode::Truck),
            "scooter" => Ok(PersonalMode::Scooter),
            _ => Err(RequestError::UnknownPersonalMode(s.to_string())),
        }
    }
}

#[derive(Debug, Error)]
pub enum RequestError {
    #[error("Missing authetification token")]
    MissingAuthToken,
    #[error("Missing location name")]
    MissingLocationName,
    #[error("Missing from and to ids")]
    MissingFromAndToId,
    #[error("Missing from id")]
    MissingFromId,
    #[error("Missing to id")]
    MissingToId,
    #[error("Unknown request type: must be LocationInformation, Trip, or StopEvent")]
    UnknownRequestType,
    #[error("Events type is not implemented")]
    EventsRequestTypeNotImplemented,
    #[error("Invalid number of results, got {0}, should be > 0.")]
    InvalidNumberResults(u32),
    #[error("Unknown personal mode: {0}")]
    UnknownPersonalMode(String),
    #[error("Http request error: {0}")]
    ReqwestError(#[from] reqwest::Error),
    #[error(
        "{0} is not a valid local time for the current timezone offset (falls in a DST gap or overlap)"
    )]
    InvalidLocalDateTime(NaiveDateTime),
}

impl TryFrom<RequestType> for String {
    type Error = RequestError;
    fn try_from(value: RequestType) -> Result<Self, Self::Error> {
        match value {
            RequestType::LocationInformation => Ok("OJPLocationInformationRequest".to_string()),
            RequestType::Trip => Ok("OJPTripRequest".to_string()),
            RequestType::StopEvent => Ok("OJPStopEventRequest".to_string()),
            RequestType::Unknown => Err(RequestError::UnknownRequestType),
        }
    }
}

#[derive(Debug)]
pub struct RequestBuilder {
    token: Option<SecretString>,
    date_time: DateTime<Utc>,
    request_type: RequestType,
    number_results: u32,
    from: Option<i32>,
    to: Option<i32>,
    name: Option<String>,
    requestor_ref: String,
    it_modes: Vec<PersonalMode>,
}

impl RequestBuilder {
    pub fn try_new(date_time: NaiveDateTime) -> Result<Self, RequestError> {
        // We convert NaiveDateTime to Utc through Local (for the offset). `Local` resolves the
        // offset that applies to `date_time` itself (accounting for DST rules across the year),
        // rather than the offset currently in effect.
        let local_date_time = date_time
            .and_local_timezone(Local)
            .single()
            .ok_or(RequestError::InvalidLocalDateTime(date_time))?;

        let date_time = local_date_time.to_utc();
        Ok(RequestBuilder {
            date_time,
            token: None,
            request_type: RequestType::Unknown,
            number_results: 0,
            from: None,
            to: None,
            name: None,
            requestor_ref: String::new(),
            it_modes: Vec::new(),
        })
    }

    pub fn set_from(mut self, from: i32) -> Self {
        self.from = Some(from);
        self
    }

    pub fn set_to(mut self, to: i32) -> Self {
        self.to = Some(to);
        self
    }

    pub fn set_token(mut self, token: SecretString) -> Self {
        self.token = Some(token);
        self
    }

    pub fn set_request_type(mut self, request_type: RequestType) -> Self {
        self.request_type = request_type;
        self
    }

    pub fn set_number_results(mut self, number_results: u32) -> Self {
        self.number_results = number_results;
        self
    }

    pub fn set_name(mut self, name: &str) -> Self {
        self.name = Some(name.to_string());
        self
    }

    pub fn set_requestor_ref(mut self, requestor_ref: &str) -> Self {
        self.requestor_ref = requestor_ref.to_string();
        self
    }

    /// Requests one additional monomodal trip per mode (walk, bike, car, ...) alongside
    /// the public-transport results of a Trip request. Ignored for other request types.
    ///
    /// Note: the Swiss OJP 2.0 endpoint has been observed to honour only a single
    /// `ItModeToCover` per request; pass one mode (or send one request per mode) if you
    /// need reliable results.
    pub fn set_it_modes(mut self, it_modes: &[PersonalMode]) -> Self {
        self.it_modes = it_modes.to_vec();
        self
    }

    pub fn try_request_body(&self) -> Result<String, RequestError> {
        let now = Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);
        let date_time = self.date_time.to_rfc3339_opts(SecondsFormat::Millis, true);

        let number_results = self.number_results;
        match self.request_type {
            RequestType::Unknown => Err(RequestError::UnknownRequestType),
            RequestType::LocationInformation => {
                if number_results == 0 {
                    return Err(RequestError::InvalidNumberResults(number_results));
                }
                let Some(name) = self.name.as_ref() else {
                    return Err(RequestError::MissingLocationName);
                };
                let requestor_ref = quick_xml::escape::escape(self.requestor_ref.as_str());
                let name = quick_xml::escape::escape(name.as_str());
                let req = format!(
"<?xml version=\"1.0\" encoding=\"UTF-8\"?>
                            <OJP xmlns=\"http://www.vdv.de/ojp\" xmlns:siri=\"http://www.siri.org.uk/siri\" version=\"2.0\">
                             	<OJPRequest>
                                    <siri:ServiceRequest>
                                        <siri:RequestTimestamp>{now}</siri:RequestTimestamp>
                                        <siri:RequestorRef>{requestor_ref}</siri:RequestorRef>
                                        <OJPLocationInformationRequest>
                                        <siri:RequestTimestamp>{now}</siri:RequestTimestamp>
                                        <siri:MessageIdentifier>LIR-1a</siri:MessageIdentifier>
                                        <InitialInput>
                                            <Name>{name}</Name>
                                        </InitialInput>
                                        <Restrictions>
                                            <Type>stop</Type>
                                            <NumberOfResults>{number_results}</NumberOfResults>
                                        </Restrictions>
                                    </OJPLocationInformationRequest>
                                    </siri:ServiceRequest>
                                </OJPRequest>
                            </OJP>");
                Ok(req)
            }
            RequestType::StopEvent => Err(RequestError::EventsRequestTypeNotImplemented),
            RequestType::Trip => {
                if number_results == 0 {
                    return Err(RequestError::InvalidNumberResults(number_results));
                }
                let (from, to) = match (self.from, self.to) {
                    (Some(from), Some(to)) => (from, to),
                    (None, None) => return Err(RequestError::MissingFromAndToId),
                    (Some(_), None) => return Err(RequestError::MissingToId),
                    (None, Some(_)) => return Err(RequestError::MissingFromId),
                };
                let requestor_ref = quick_xml::escape::escape(self.requestor_ref.as_str());
                // Values come from the closed PersonalMode enum, so no escaping is needed.
                // Schema order (TripPolicyGroup): ItModeToCover follows NumberOfResults.
                let it_modes = self
                    .it_modes
                    .iter()
                    .map(|m| {
                        format!("<ItModeToCover><PersonalMode>{m}</PersonalMode></ItModeToCover>")
                    })
                    .collect::<String>();
                let req = format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?>
                            <OJP xmlns=\"http://www.vdv.de/ojp\" xmlns:siri=\"http://www.siri.org.uk/siri\" version=\"2.0\">
                             	<OJPRequest>
                                    <siri:ServiceRequest>
                                        <siri:RequestTimestamp>{now}</siri:RequestTimestamp>
                                        <siri:RequestorRef>{requestor_ref}</siri:RequestorRef>
                                        <OJPTripRequest>
                                            <siri:RequestTimestamp>{now}</siri:RequestTimestamp>
                                            <siri:MessageIdentifier>TR-1r1</siri:MessageIdentifier>
                                            <Origin>
                                                <PlaceRef>
                                                    <siri:StopPointRef>{from}</siri:StopPointRef>
                                                </PlaceRef>
                                                <DepArrTime>{date_time}</DepArrTime>
                                            </Origin>
                                            <Destination>
                                                <PlaceRef>
                                                    <siri:StopPointRef>{to}</siri:StopPointRef>
                                                </PlaceRef>
                                            </Destination>
                                            <Params>
                                                <NumberOfResults>{number_results}</NumberOfResults>
                                                {it_modes}
                                            </Params>
                                        </OJPTripRequest>
                                    </siri:ServiceRequest>
                                </OJPRequest>
                            </OJP>");
                Ok(req)
            }
        }
    }

    pub fn build_request(self) -> Result<reqwest::RequestBuilder, RequestError> {
        let id_request = self.try_request_body()?;

        if self.token.is_none() {
            return Err(RequestError::MissingAuthToken);
        }
        {
            let span = span!(Level::INFO, "Performing OJP request");
            let _guard = span.enter();
            tracing::info!("{}", self);
        }
        let token = self.token.ok_or(RequestError::MissingAuthToken)?;

        let req = Client::new()
            .post(URL)
            .header("Content-Type", "application/xml")
            .header("accept", "*/*")
            .bearer_auth(token.expose_secret())
            .body(id_request);

        Ok(req)
    }

    pub async fn send_request(self) -> Result<String, RequestError> {
        let respone = self.build_request()?.send().await?.text().await?;
        Ok(respone)
    }
}

impl Display for RequestBuilder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let token = self
            .token
            .as_ref()
            .map(|s| format!("{:?}", s))
            .unwrap_or("Undefined".to_string());

        match self.request_type {
            RequestType::LocationInformation => {
                write!(f, "Location Information Request: ")?;
                write!(
                    f,
                    "Location: {}, ",
                    self.name
                        .as_ref()
                        .map(|i| i.to_string())
                        .unwrap_or("Undefined".to_string()),
                )?;
            }
            RequestType::Trip => {
                write!(f, "Trip Request: ")?;
                write!(
                    f,
                    "From: {}, To: {}, ",
                    self.from
                        .map(|i| format!("{i}"))
                        .unwrap_or("Undefined".to_string()),
                    self.to
                        .map(|i| format!("{i}"))
                        .unwrap_or("Undefined".to_string()),
                )?;
                if !self.it_modes.is_empty() {
                    write!(f, "ItModes: {:?}, ", self.it_modes)?;
                }
            }
            RequestType::StopEvent => {
                write!(f, "Stop Envent Request Not Implement. ")?;
            }
            RequestType::Unknown => {
                write!(f, "RequestType is unknown. ")?;
            }
        }
        write!(
            f,
            "NumberResults: {}, DateTime: {}, RequestorRef: {}, Token: {token}",
            self.number_results, self.date_time, self.requestor_ref
        )
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use chrono::{NaiveDate, NaiveTime};

    fn sample_date_time() -> NaiveDateTime {
        NaiveDateTime::new(
            NaiveDate::from_ymd_opt(2025, 7, 15).unwrap(),
            NaiveTime::from_hms_opt(10, 0, 0).unwrap(),
        )
    }

    #[test]
    fn location_request_escapes_special_characters() {
        let body = RequestBuilder::try_new(sample_date_time())
            .unwrap()
            .set_request_type(RequestType::LocationInformation)
            .set_requestor_ref("<ref>&\"'")
            .set_name("<name>&\"'")
            .set_number_results(1)
            .try_request_body()
            .unwrap();

        assert!(
            body.contains("<siri:RequestorRef>&lt;ref&gt;&amp;&quot;&apos;</siri:RequestorRef>")
        );
        assert!(body.contains("<Name>&lt;name&gt;&amp;&quot;&apos;</Name>"));
    }

    #[test]
    fn location_request_name_cannot_inject_sibling_elements() {
        // Attempts to close </Name></InitialInput> early and splice in a sibling element.
        let malicious = "</Name></InitialInput><Injected>pwned</Injected><InitialInput><Name>x";
        let body = RequestBuilder::try_new(sample_date_time())
            .unwrap()
            .set_request_type(RequestType::LocationInformation)
            .set_requestor_ref("ref")
            .set_name(malicious)
            .set_number_results(1)
            .try_request_body()
            .unwrap();

        assert!(!body.contains("<Injected>"));
        assert_eq!(body.matches("<InitialInput>").count(), 1);
    }

    #[test]
    fn trip_request_escapes_requestor_ref() {
        let body = RequestBuilder::try_new(sample_date_time())
            .unwrap()
            .set_request_type(RequestType::Trip)
            .set_requestor_ref("<ref>&")
            .set_from(1)
            .set_to(2)
            .set_number_results(1)
            .try_request_body()
            .unwrap();

        assert!(body.contains("<siri:RequestorRef>&lt;ref&gt;&amp;</siri:RequestorRef>"));
    }

    #[test]
    fn trip_request_includes_it_modes_after_number_of_results() {
        let body = RequestBuilder::try_new(sample_date_time())
            .unwrap()
            .set_request_type(RequestType::Trip)
            .set_from(1)
            .set_to(2)
            .set_number_results(3)
            .set_it_modes(&[PersonalMode::Bicycle, PersonalMode::Car])
            .try_request_body()
            .unwrap();

        assert!(
            body.contains("<ItModeToCover><PersonalMode>bicycle</PersonalMode></ItModeToCover>")
        );
        assert!(body.contains("<ItModeToCover><PersonalMode>car</PersonalMode></ItModeToCover>"));
        // The schema requires ItModeToCover to come after NumberOfResults inside Params.
        let number_results_pos = body.find("<NumberOfResults>").unwrap();
        let it_mode_pos = body.find("<ItModeToCover>").unwrap();
        assert!(number_results_pos < it_mode_pos);
    }

    /// Every PersonalMode variant. A new variant must be added here.
    const ALL_MODES: [PersonalMode; 6] = [
        PersonalMode::Foot,
        PersonalMode::Bicycle,
        PersonalMode::Car,
        PersonalMode::Motorcycle,
        PersonalMode::Truck,
        PersonalMode::Scooter,
    ];

    #[test]
    fn personal_mode_string_round_trip() {
        for mode in ALL_MODES {
            assert_eq!(mode.to_string().parse::<PersonalMode>().unwrap(), mode);
        }
        // Pin the wire values of the OJP 2.0 PersonalModesEnumeration in one place:
        // a round trip alone would still pass if `as_str` and `from_str` drifted from
        // the spec together.
        assert_eq!(
            ALL_MODES.map(PersonalMode::as_str),
            ["foot", "bicycle", "car", "motorcycle", "truck", "scooter"]
        );
        assert!(matches!(
            "plane".parse::<PersonalMode>(),
            Err(RequestError::UnknownPersonalMode(s)) if s == "plane"
        ));
    }

    #[test]
    fn trip_request_emits_every_personal_mode() {
        let body = RequestBuilder::try_new(sample_date_time())
            .unwrap()
            .set_request_type(RequestType::Trip)
            .set_from(1)
            .set_to(2)
            .set_number_results(3)
            .set_it_modes(&ALL_MODES)
            .try_request_body()
            .unwrap();

        for mode in ALL_MODES {
            assert!(
                body.contains(&format!(
                    "<ItModeToCover><PersonalMode>{mode}</PersonalMode></ItModeToCover>"
                )),
                "missing ItModeToCover for {mode:?}"
            );
        }
        assert_eq!(body.matches("<ItModeToCover>").count(), ALL_MODES.len());
    }

    #[test]
    fn trip_request_without_it_modes_omits_element() {
        let body = RequestBuilder::try_new(sample_date_time())
            .unwrap()
            .set_request_type(RequestType::Trip)
            .set_from(1)
            .set_to(2)
            .set_number_results(3)
            .try_request_body()
            .unwrap();

        assert!(!body.contains("ItModeToCover"));
    }

    #[test]
    fn try_new_resolves_offset_for_given_date_not_now() {
        // Regression test: try_new must resolve the UTC offset that applies to `date_time`
        // itself (via `Local`), not the offset currently in effect (via `Local::now()`).
        let date_time = sample_date_time();
        let expected_utc = date_time
            .and_local_timezone(Local)
            .single()
            .unwrap()
            .to_utc();
        let expected = expected_utc.to_rfc3339_opts(SecondsFormat::Millis, true);

        let body = RequestBuilder::try_new(date_time)
            .unwrap()
            .set_request_type(RequestType::Trip)
            .set_from(1)
            .set_to(2)
            .set_number_results(1)
            .try_request_body()
            .unwrap();

        assert!(body.contains(&format!("<DepArrTime>{expected}</DepArrTime>")));
    }
}
