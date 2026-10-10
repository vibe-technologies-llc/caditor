use std::{ffi::OsString, os::unix::ffi::OsStringExt, path::PathBuf};

pub(crate) const URI_LIST: &str = "text/uri-list";

const FILE_SCHEME: &[u8] = b"file:";
const AUTHORITY: &[u8] = b"//";
const LOCALHOST: &[u8] = b"localhost";
const COMMENT: u8 = b'#';
const ESCAPE: u8 = b'%';

pub(crate) fn file_paths(list: &[u8], local_host: &[u8]) -> Vec<PathBuf> {
    list.split(|byte| *byte == b'\n')
        .map(<[u8]>::trim_ascii)
        .filter(|line| !line.is_empty() && line.first() != Some(&COMMENT))
        .filter_map(|uri| file_path(uri, local_host))
        .collect()
}

fn file_path(uri: &[u8], local_host: &[u8]) -> Option<PathBuf> {
    let scheme = uri.get(..FILE_SCHEME.len())?;
    if !scheme.eq_ignore_ascii_case(FILE_SCHEME) {
        return None;
    }
    let rest = uri.get(FILE_SCHEME.len()..)?;
    let path = match rest.strip_prefix(AUTHORITY) {
        Some(authority) => {
            let (host, path) = authority.split_at(authority.iter().position(|byte| *byte == b'/')?);
            let local = host.is_empty()
                || host.eq_ignore_ascii_case(LOCALHOST)
                || host.eq_ignore_ascii_case(local_host);
            local.then_some(path)?
        }
        None => rest.starts_with(b"/").then_some(rest)?,
    };
    let decoded = percent_decoded(path);
    if decoded.contains(&0) {
        return None;
    }
    Some(PathBuf::from(OsString::from_vec(decoded)))
}

fn percent_decoded(text: &[u8]) -> Vec<u8> {
    let mut decoded = Vec::with_capacity(text.len());
    let mut rest = text;
    while let Some((&byte, after)) = rest.split_first() {
        let escaped = match after {
            [high, low, ..] if byte == ESCAPE => hex(*high).zip(hex(*low)),
            _ => None,
        };
        match (escaped, after.get(2..)) {
            (Some((high, low)), Some(remaining)) => {
                decoded.push((high << 4) | low);
                rest = remaining;
            }
            _ => {
                decoded.push(byte);
                rest = after;
            }
        }
    }
    decoded
}

fn hex(digit: u8) -> Option<u8> {
    char::from(digit)
        .to_digit(16)
        .and_then(|value| u8::try_from(value).ok())
}

#[cfg(test)]
mod tests {
    use std::{os::unix::ffi::OsStrExt, path::Path};

    use super::*;

    const HOST: &[u8] = b"workshop";

    #[test]
    fn a_file_manager_list_decodes_to_its_paths() {
        let list = b"file:///home/ana/part.step\r\nfile:///home/ana/My%20Sketch.dxf\r\n";

        let paths = file_paths(list, HOST);

        assert_eq!(
            paths,
            [
                PathBuf::from("/home/ana/part.step"),
                PathBuf::from("/home/ana/My Sketch.dxf")
            ]
        );
    }

    #[test]
    fn comments_blank_lines_and_bare_line_feeds_are_read() {
        let list = b"# copied from a browser\n\nfile:///tmp/a.svg\n  file:///tmp/b.svg  \n";

        let paths = file_paths(list, HOST);

        assert_eq!(
            paths,
            [PathBuf::from("/tmp/a.svg"), PathBuf::from("/tmp/b.svg")]
        );
    }

    #[test]
    fn only_files_on_this_machine_are_taken() {
        let list = b"FILE://localhost/tmp/a.dxf\r\nfile://WORKSHOP/tmp/b.dxf\r\nfile:/tmp/c.dxf\r\n\
                     file://elsewhere/tmp/d.dxf\r\nhttps://example.org/e.dxf\r\nfile:relative.dxf\r\n\
                     file://localhost";

        let paths = file_paths(list, HOST);

        assert_eq!(
            paths,
            [
                PathBuf::from("/tmp/a.dxf"),
                PathBuf::from("/tmp/b.dxf"),
                PathBuf::from("/tmp/c.dxf")
            ]
        );
    }

    #[test]
    fn escapes_keep_bytes_that_are_not_text_and_refuse_a_nul() {
        let list =
            b"file:///tmp/caf%C3%A9%FF.step\r\nfile:///tmp/100%25%zz%4\r\nfile:///tmp/a%00b\r\n";

        let paths = file_paths(list, HOST);

        assert_eq!(paths.len(), 2);
        assert_eq!(
            paths.first().map(|path| path.as_os_str().as_bytes()),
            Some(&b"/tmp/caf\xC3\xA9\xFF.step"[..])
        );
        assert_eq!(
            paths.get(1).map(PathBuf::as_path),
            Some(Path::new("/tmp/100%%zz%4"))
        );
    }
}
