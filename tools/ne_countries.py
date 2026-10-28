"""Make app/ui/vendor/countries.js: one label point per country, from Natural Earth admin-0 countries.

The route map moves the home point to the middle of its country on pictures for publication, and puts a hop
whose database record has only a country there. Natural Earth is public domain. Only points are kept: country
names come from the IP database, so the program takes no side in naming disputes.

    python tools/ne_countries.py ne_10m_admin_0_countries.geojson app/ui/vendor/countries.js

The input is the 1:10m GeoJSON of Natural Earth v5.1.2 (github.com/nvkelso/natural-earth-vector, geojson/).
"""

import json
import sys


def main(src, out):
    with open(src, encoding="utf-8") as f:
        features = json.load(f)["features"]
    best = {}
    for feature in features:
        p = feature["properties"]
        code = p.get("ISO_A2_EH")
        if not code or code == "-99":
            continue
        # A code can belong to a country and to its outlying parts; the main unit has the lowest label rank.
        if code in best and best[code][0] <= p["LABELRANK"]:
            continue
        best[code] = (p["LABELRANK"], round(p["LABEL_X"], 2), round(p["LABEL_Y"], 2))
    rows = ",".join('"%s":[%s,%s]' % (code, best[code][1], best[code][2]) for code in sorted(best))
    with open(out, "w", encoding="utf-8", newline="\n") as f:
        f.write(
            "// Country label points, ISO 3166-1 alpha-2 code -> [longitude, latitude]: Natural Earth 1:10m admin-0\n"
            "// countries v5.1.2 (public domain), fields ISO_A2_EH, LABEL_X, LABEL_Y. Made by tools/ne_countries.py.\n"
            "window.WT_COUNTRY_POINTS = {" + rows + "};\n"
        )
    print(len(best), "countries")


if __name__ == "__main__":
    main(sys.argv[1], sys.argv[2])
