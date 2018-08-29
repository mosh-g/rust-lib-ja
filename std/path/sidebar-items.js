initSidebarItems({"constant":[["MAIN_SEPARATOR","The primary separator of path components for the current platform."]],"enum":[["Component","A single component of a path."],["Prefix","Windows path prefixes, e.g. `C:` or `\\\\server\\share`."]],"fn":[["is_separator","Determines whether the character is one of the permitted path separators for the current platform."]],"struct":[["Ancestors","An iterator over [`Path`] and its ancestors."],["Components","An iterator over the [`Component`]s of a [`Path`]."],["Display","Helper struct for safely printing paths with [`format!`] and `{}`."],["Iter","An iterator over the [`Component`]s of a [`Path`], as [`OsStr`] slices."],["Path","A slice of a path (akin to [`str`])."],["PathBuf","An owned, mutable path (akin to [`String`])."],["PrefixComponent","A structure wrapping a Windows path prefix as well as its unparsed string representation."],["StripPrefixError","An error returned from [`Path::strip_prefix`][`strip_prefix`] if the prefix was not found."]]});