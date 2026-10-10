use std::{
    io::{self, PipeReader, Read},
    os::fd::AsFd,
    time::{Duration, Instant},
};

use rustix::event::{PollFd, PollFlags, Timespec, poll};

pub(crate) const MOST_BYTES: usize = 1 << 20;
pub(crate) const PATIENCE: Duration = Duration::from_secs(5);

const CHUNK: usize = 4096;

#[derive(Debug, thiserror::Error)]
pub(crate) enum ReadError {
    #[error("the list of dragged files could not be read: {0}")]
    Io(#[from] io::Error),
    #[error("the application the files are dragged from sent no list within {0:?}")]
    TimedOut(Duration),
    #[error("the list of dragged files is longer than {MOST_BYTES} bytes")]
    TooLong,
}

pub(crate) fn read_whole(mut reader: PipeReader, patience: Duration) -> Result<Vec<u8>, ReadError> {
    let started = Instant::now();
    let mut list = Vec::new();
    let mut chunk = [0; CHUNK];
    loop {
        let left = patience
            .checked_sub(started.elapsed())
            .ok_or(ReadError::TimedOut(patience))?;
        if !readable_within(&reader, left)? {
            return Err(ReadError::TimedOut(patience));
        }
        let count = match reader.read(&mut chunk) {
            Ok(0) => return Ok(list),
            Ok(count) => count,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error.into()),
        };
        list.extend(chunk.get(..count).unwrap_or_default());
        if list.len() > MOST_BYTES {
            return Err(ReadError::TooLong);
        }
    }
}

fn readable_within(reader: &PipeReader, left: Duration) -> io::Result<bool> {
    let timeout = Timespec::try_from(left).map_err(io::Error::other)?;
    let mut watched = [PollFd::from_borrowed_fd(reader.as_fd(), PollFlags::IN)];
    loop {
        match poll(&mut watched, Some(&timeout)) {
            Ok(ready) => return Ok(ready > 0),
            Err(rustix::io::Errno::INTR) => continue,
            Err(error) => return Err(error.into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{io::Write, thread};

    use super::*;

    #[test]
    fn the_whole_list_is_read_once_the_writer_closes() {
        let (reader, mut writer) = io::pipe().unwrap();
        let list = "file:///tmp/a.step\r\n".repeat(1000);
        let sent = list.clone();

        let writing = thread::spawn(move || writer.write_all(sent.as_bytes()).unwrap());
        let read = read_whole(reader, PATIENCE).unwrap();
        writing.join().unwrap();

        assert_eq!(read, list.as_bytes());
    }

    #[test]
    fn a_writer_that_never_closes_times_out() {
        let (reader, mut writer) = io::pipe().unwrap();
        writer.write_all(b"file:///tmp/a.step").unwrap();

        let read = read_whole(reader, Duration::from_millis(30));

        assert!(matches!(read, Err(ReadError::TimedOut(_))));
        drop(writer);
    }
}
