#!/bin/sh
# Downloads veraPDF (the validator for PDF/A and PDF/UA) and its dependencies from Maven Central.
#
#     tools/fetch_verapdf.sh DIRECTORY
#
# Needs Java and Maven. The jars are in DIRECTORY/lib; with `VERAPDF_LIB=DIRECTORY/lib` the test
# `tests/conformance.rs` validates the gallery with them (it is skipped without).
set -e
dir="${1:?usage: fetch_verapdf.sh DIRECTORY}"
version=1.28.2
mkdir -p "$dir"
cat > "$dir/pom.xml" <<POM
<project xmlns="http://maven.apache.org/POM/4.0.0">
  <modelVersion>4.0.0</modelVersion>
  <groupId>local</groupId><artifactId>verapdf-fetch</artifactId><version>1</version>
  <dependencies>
    <dependency>
      <groupId>org.verapdf.apps</groupId><artifactId>greenfield-apps</artifactId><version>$version</version>
    </dependency>
  </dependencies>
</project>
POM
(cd "$dir" && mvn -q -B dependency:copy-dependencies -DoutputDirectory=lib)
echo "veraPDF $version is in $dir/lib"
