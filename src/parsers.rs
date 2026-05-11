use crate::domain;
use nom::branch::alt;
use nom::bytes::complete::take_while1;
use nom::bytes::complete::{tag, take_while};
use nom::character::complete::multispace0;
use nom::character::complete::u64;
use nom::error::Error;
use nom::multi::{many0, separated_list1};
use nom::sequence::{delimited, preceded, terminated};
use nom::Parser;
use nom::IResult;
use crate::domain::{GoodId, SourceId};

fn unicode_ws(input: &str) -> IResult<&str, &str> {
    take_while(|c: char| c.is_whitespace())(input)
}

pub fn count(input: &str) -> IResult<&str, u64> {
    delimited(tag("("), u64, tag(")")).parse(input)
}

pub fn good_id(input: &str) -> IResult<&str, GoodId> {
    let (input, name) = take_while1(|c: char| c != '(').parse(input)?;
    Ok((input, GoodId(name.trim_matches(|c: char| c.is_whitespace()).into())))
}

pub fn source_id(input: &str) -> IResult<&str, SourceId> {
    let (input, name) = take_while1(|c: char| c != '(')(input)?;
    Ok((input, SourceId(name.trim_matches(|c: char| c.is_whitespace()).into())))
}

pub fn need(input: &str) -> IResult<&str, (GoodId, u64)> {
    let (input, _) = unicode_ws(input)?;
    let (input, good_id) = good_id(input)?;
    let (input, _) = unicode_ws(input)?;
    let (input, quantity) = count(input)?;

    Ok((input, (good_id, quantity)))
}

pub fn needs(input: &str) -> IResult<&str, Vec<(domain::GoodId, u64)>> {
    let (input, _) = multispace0(input)?;
    many0(need).parse(input)
}

#[derive(Debug, Clone)]
pub enum TimeUnit {
    Seconds(u64),
    Minutes(u64),
    Hours(u64),
    Days(u64),
}

pub fn time(input: &str) -> IResult<&str, Vec<TimeUnit>> {
    fn time_unit<'a, F>(input: &'a str, kind_parser: F) -> IResult<&'a str, u64>
    where
        F: Parser<&'a str, Output = &'a str, Error = Error<&'a str>>,
    {
        let (reminder, t) = terminated(u64, preceded(multispace0, kind_parser)).parse(input)?;
        Ok((reminder, t))
    }

    fn seconds(input: &str) -> IResult<&str, TimeUnit> {
        let (reminder, t) = time_unit(input, alt((tag("seconds"), tag("s"))))?;
        Ok((reminder, TimeUnit::Seconds(t)))
    }

    fn minutes(input: &str) -> IResult<&str, TimeUnit> {
        let (reminder, t) = time_unit(input, alt((tag("minutes"), tag("min"))))?;
        Ok((reminder, TimeUnit::Minutes(t)))
    }

    fn hours(input: &str) -> IResult<&str, TimeUnit> {
        let (reminder, t) = time_unit(input, alt((tag("hours"), tag("h"))))?;
        Ok((reminder, TimeUnit::Hours(t)))
    }

    fn days(input: &str) -> IResult<&str, TimeUnit> {
        let (reminder, t) = time_unit(input, alt((tag("days"), tag("d"))))?;
        Ok((reminder, TimeUnit::Days(t)))
    }

    let (reminder, times) =
        separated_list1(multispace0, alt((seconds, minutes, hours, days))).parse(input)?;

    Ok((reminder, times))
}
