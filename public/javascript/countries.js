/**
 * Ország-, nyelv- és ISO kód adatbázis és segédmodul az EPG Viewerhez.
 *
 * Minden ország tartalmazza:
 * - code: rövidített nyelvi/országkód (HU, EN, US, AT, DE, stb.)
 * - isoCode: BCP 47 / ISO szabványos nyelvi-területi kód (hu-HU, en-US, de-AT, stb.)
 * - nameHu: magyar megnevezés
 * - nameEn: angol megnevezés
 * - nativeName: anyanyelvi megnevezés
 * - flag: zászló emoji
 * - aliases: alternatív kódok (pl. EN/GB -> UK)
 *
 * Sorrend: 1. EN, 2. HU, a többi A-Z szerint rendezve.
 */

(function () {
  'use strict';

  const COUNTRIES = [
  {
    "code": "EN",
    "isoCode": "en-US",
    "nameHu": "Angol (Nemzetközi)",
    "nameEn": "English (International)",
    "nativeName": "English",
    "flag": "🌐",
    "aliases": [
      "ENG",
      "GLOBAL"
    ]
  },
  {
    "code": "HU",
    "isoCode": "hu-HU",
    "nameHu": "Magyarország",
    "nameEn": "Hungary",
    "nativeName": "Magyar",
    "flag": "🇭🇺",
    "aliases": [
      "HUN",
      "HUNGARIAN"
    ]
  },
  {
    "code": "AD",
    "isoCode": "ca-AD",
    "nameHu": "Andorra",
    "nameEn": "Andorra",
    "nativeName": "Català",
    "flag": "🇦🇩",
    "aliases": [
      "AND"
    ]
  },
  {
    "code": "AE",
    "isoCode": "ar-AE",
    "nameHu": "Egyesült Arab Emírségek",
    "nameEn": "United Arab Emirates",
    "nativeName": "العربية",
    "flag": "🇦🇪",
    "aliases": [
      "ARE",
      "UAE"
    ]
  },
  {
    "code": "AF",
    "isoCode": "fa-AF",
    "nameHu": "Afganisztán",
    "nameEn": "Afghanistan",
    "nativeName": "فارسی / پښتو",
    "flag": "🇦🇫",
    "aliases": [
      "AFG"
    ]
  },
  {
    "code": "AG",
    "isoCode": "en-AG",
    "nameHu": "Antigua és Barbuda",
    "nameEn": "Antigua and Barbuda",
    "nativeName": "English",
    "flag": "🇦🇬",
    "aliases": [
      "ATG"
    ]
  },
  {
    "code": "AI",
    "isoCode": "en-AI",
    "nameHu": "Anguilla",
    "nameEn": "Anguilla",
    "nativeName": "English",
    "flag": "🇦🇮",
    "aliases": [
      "AIA"
    ]
  },
  {
    "code": "AL",
    "isoCode": "sq-AL",
    "nameHu": "Albánia",
    "nameEn": "Albania",
    "nativeName": "Shqip",
    "flag": "🇦🇱",
    "aliases": [
      "ALB"
    ]
  },
  {
    "code": "AM",
    "isoCode": "hy-AM",
    "nameHu": "Örményország",
    "nameEn": "Armenia",
    "nativeName": "Հայերեն",
    "flag": "🇦🇲",
    "aliases": [
      "ARM"
    ]
  },
  {
    "code": "AO",
    "isoCode": "pt-AO",
    "nameHu": "Angola",
    "nameEn": "Angola",
    "nativeName": "Português",
    "flag": "🇦🇴",
    "aliases": [
      "AGO"
    ]
  },
  {
    "code": "AQ",
    "isoCode": "en-AQ",
    "nameHu": "Antarktisz",
    "nameEn": "Antarctica",
    "nativeName": "English",
    "flag": "🇦🇶",
    "aliases": [
      "ATA"
    ]
  },
  {
    "code": "AR",
    "isoCode": "es-AR",
    "nameHu": "Argentína",
    "nameEn": "Argentina",
    "nativeName": "Español",
    "flag": "🇦🇷",
    "aliases": [
      "ARG"
    ]
  },
  {
    "code": "AS",
    "isoCode": "en-AS",
    "nameHu": "Amerikai Szamoa",
    "nameEn": "American Samoa",
    "nativeName": "English / Gagana Sāmoa",
    "flag": "🇦🇸",
    "aliases": [
      "ASM"
    ]
  },
  {
    "code": "AT",
    "isoCode": "de-AT",
    "nameHu": "Ausztria",
    "nameEn": "Austria",
    "nativeName": "Deutsch (Österreich)",
    "flag": "🇦🇹",
    "aliases": [
      "AUT"
    ]
  },
  {
    "code": "AU",
    "isoCode": "en-AU",
    "nameHu": "Ausztrália",
    "nameEn": "Australia",
    "nativeName": "English (AU)",
    "flag": "🇦🇺",
    "aliases": [
      "AUS"
    ]
  },
  {
    "code": "AW",
    "isoCode": "nl-AW",
    "nameHu": "Aruba",
    "nameEn": "Aruba",
    "nativeName": "Nederlands / Papiamento",
    "flag": "🇦🇼",
    "aliases": [
      "ABW"
    ]
  },
  {
    "code": "AX",
    "isoCode": "sv-AX",
    "nameHu": "Åland-szigetek",
    "nameEn": "Åland Islands",
    "nativeName": "Svenska",
    "flag": "🇦🇽",
    "aliases": [
      "ALA"
    ]
  },
  {
    "code": "AZ",
    "isoCode": "az-AZ",
    "nameHu": "Azerbajdzsán",
    "nameEn": "Azerbaijan",
    "nativeName": "Azərbaycan",
    "flag": "🇦🇿",
    "aliases": [
      "AZE"
    ]
  },
  {
    "code": "BA",
    "isoCode": "bs-BA",
    "nameHu": "Bosznia-Hercegovina",
    "nameEn": "Bosnia and Herzegovina",
    "nativeName": "Bosanski / Hrvatski / Srpski",
    "flag": "🇧🇦",
    "aliases": [
      "BIH"
    ]
  },
  {
    "code": "BB",
    "isoCode": "en-BB",
    "nameHu": "Barbados",
    "nameEn": "Barbados",
    "nativeName": "English",
    "flag": "🇧🇧",
    "aliases": [
      "BRB"
    ]
  },
  {
    "code": "BD",
    "isoCode": "bn-BD",
    "nameHu": "Banglades",
    "nameEn": "Bangladesh",
    "nativeName": "বাংলা",
    "flag": "🇧🇩",
    "aliases": [
      "BGD"
    ]
  },
  {
    "code": "BE",
    "isoCode": "nl-BE",
    "nameHu": "Belgium",
    "nameEn": "Belgium",
    "nativeName": "Nederlands / Français",
    "flag": "🇧🇪",
    "aliases": [
      "BEL"
    ]
  },
  {
    "code": "BF",
    "isoCode": "fr-BF",
    "nameHu": "Burkina Faso",
    "nameEn": "Burkina Faso",
    "nativeName": "Français",
    "flag": "🇧🇫",
    "aliases": [
      "BFA"
    ]
  },
  {
    "code": "BG",
    "isoCode": "bg-BG",
    "nameHu": "Bulgária",
    "nameEn": "Bulgaria",
    "nativeName": "Български",
    "flag": "🇧🇬",
    "aliases": [
      "BGR"
    ]
  },
  {
    "code": "BH",
    "isoCode": "ar-BH",
    "nameHu": "Bahrein",
    "nameEn": "Bahrain",
    "nativeName": "العربية",
    "flag": "🇧🇭",
    "aliases": [
      "BHR"
    ]
  },
  {
    "code": "BI",
    "isoCode": "rn-BI",
    "nameHu": "Burundi",
    "nameEn": "Burundi",
    "nativeName": "Kirundi / Français",
    "flag": "🇧🇮",
    "aliases": [
      "BDI"
    ]
  },
  {
    "code": "BJ",
    "isoCode": "fr-BJ",
    "nameHu": "Benin",
    "nameEn": "Benin",
    "nativeName": "Français",
    "flag": "🇧🇯",
    "aliases": [
      "BEN"
    ]
  },
  {
    "code": "BL",
    "isoCode": "fr-BL",
    "nameHu": "Saint-Barthélemy",
    "nameEn": "Saint Barthélemy",
    "nativeName": "Français",
    "flag": "🇧🇱",
    "aliases": [
      "BLM"
    ]
  },
  {
    "code": "BM",
    "isoCode": "en-BM",
    "nameHu": "Bermuda",
    "nameEn": "Bermuda",
    "nativeName": "English",
    "flag": "🇧🇲",
    "aliases": [
      "BMU"
    ]
  },
  {
    "code": "BN",
    "isoCode": "ms-BN",
    "nameHu": "Brunei",
    "nameEn": "Brunei",
    "nativeName": "Bahasa Melayu",
    "flag": "🇧🇳",
    "aliases": [
      "BRN"
    ]
  },
  {
    "code": "BO",
    "isoCode": "es-BO",
    "nameHu": "Bolívia",
    "nameEn": "Bolivia",
    "nativeName": "Español / Quechua / Aymara",
    "flag": "🇧🇴",
    "aliases": [
      "BOL"
    ]
  },
  {
    "code": "BQ",
    "isoCode": "nl-BQ",
    "nameHu": "Bonaire, Sint Eustatius és Saba",
    "nameEn": "Caribbean Netherlands",
    "nativeName": "Nederlands / Papiamento",
    "flag": "🇧🇶",
    "aliases": [
      "BES"
    ]
  },
  {
    "code": "BR",
    "isoCode": "pt-BR",
    "nameHu": "Brazília",
    "nameEn": "Brazil",
    "nativeName": "Português (Brasil)",
    "flag": "🇧🇷",
    "aliases": [
      "BRA"
    ]
  },
  {
    "code": "BS",
    "isoCode": "en-BS",
    "nameHu": "Bahama-szigetek",
    "nameEn": "Bahamas",
    "nativeName": "English",
    "flag": "🇧🇸",
    "aliases": [
      "BHS"
    ]
  },
  {
    "code": "BT",
    "isoCode": "dz-BT",
    "nameHu": "Bhután",
    "nameEn": "Bhutan",
    "nativeName": "རྫོང་ཁ",
    "flag": "🇧🇹",
    "aliases": [
      "BTN"
    ]
  },
  {
    "code": "BV",
    "isoCode": "no-BV",
    "nameHu": "Bouvet-sziget",
    "nameEn": "Bouvet Island",
    "nativeName": "Norsk",
    "flag": "🇧🇻",
    "aliases": [
      "BVT"
    ]
  },
  {
    "code": "BW",
    "isoCode": "en-BW",
    "nameHu": "Botswana",
    "nameEn": "Botswana",
    "nativeName": "English / Setswana",
    "flag": "🇧🇼",
    "aliases": [
      "BWA"
    ]
  },
  {
    "code": "BY",
    "isoCode": "be-BY",
    "nameHu": "Fehéroroszország",
    "nameEn": "Belarus",
    "nativeName": "Беларуская / Русский",
    "flag": "🇧🇾",
    "aliases": [
      "BLR"
    ]
  },
  {
    "code": "BZ",
    "isoCode": "en-BZ",
    "nameHu": "Belize",
    "nameEn": "Belize",
    "nativeName": "English",
    "flag": "🇧🇿",
    "aliases": [
      "BLZ"
    ]
  },
  {
    "code": "CA",
    "isoCode": "en-CA",
    "nameHu": "Kanada",
    "nameEn": "Canada",
    "nativeName": "English / Français",
    "flag": "🇨🇦",
    "aliases": [
      "CAN"
    ]
  },
  {
    "code": "CC",
    "isoCode": "en-CC",
    "nameHu": "Kókusz-szigetek",
    "nameEn": "Cocos (Keeling) Islands",
    "nativeName": "English",
    "flag": "🇨🇨",
    "aliases": [
      "CCK"
    ]
  },
  {
    "code": "CD",
    "isoCode": "fr-CD",
    "nameHu": "Kongói Demokratikus Köztársaság",
    "nameEn": "DR Congo",
    "nativeName": "Français / Lingála",
    "flag": "🇨🇩",
    "aliases": [
      "COD"
    ]
  },
  {
    "code": "CF",
    "isoCode": "fr-CF",
    "nameHu": "Közép-afrikai Köztársaság",
    "nameEn": "Central African Republic",
    "nativeName": "Français / Sängö",
    "flag": "🇨🇫",
    "aliases": [
      "CAF"
    ]
  },
  {
    "code": "CG",
    "isoCode": "fr-CG",
    "nameHu": "Kongói Köztársaság",
    "nameEn": "Republic of the Congo",
    "nativeName": "Français",
    "flag": "🇨🇬",
    "aliases": [
      "COG"
    ]
  },
  {
    "code": "CH",
    "isoCode": "de-CH",
    "nameHu": "Svájc",
    "nameEn": "Switzerland",
    "nativeName": "Deutsch / Français / Italiano",
    "flag": "🇨🇭",
    "aliases": [
      "SUI",
      "CHE"
    ]
  },
  {
    "code": "CI",
    "isoCode": "fr-CI",
    "nameHu": "Elefántcsontpart",
    "nameEn": "Ivory Coast",
    "nativeName": "Français",
    "flag": "🇨🇮",
    "aliases": [
      "CIV"
    ]
  },
  {
    "code": "CK",
    "isoCode": "en-CK",
    "nameHu": "Cook-szigetek",
    "nameEn": "Cook Islands",
    "nativeName": "English",
    "flag": "🇨🇰",
    "aliases": [
      "COK"
    ]
  },
  {
    "code": "CL",
    "isoCode": "es-CL",
    "nameHu": "Chile",
    "nameEn": "Chile",
    "nativeName": "Español",
    "flag": "🇨🇱",
    "aliases": [
      "CHL"
    ]
  },
  {
    "code": "CM",
    "isoCode": "fr-CM",
    "nameHu": "Kamerun",
    "nameEn": "Cameroon",
    "nativeName": "Français / English",
    "flag": "🇨🇲",
    "aliases": [
      "CMR"
    ]
  },
  {
    "code": "CN",
    "isoCode": "zh-CN",
    "nameHu": "Kína",
    "nameEn": "China",
    "nativeName": "中文 (简体)",
    "flag": "🇨🇳",
    "aliases": [
      "CHN"
    ]
  },
  {
    "code": "CO",
    "isoCode": "es-CO",
    "nameHu": "Kolumbia",
    "nameEn": "Colombia",
    "nativeName": "Español",
    "flag": "🇨🇴",
    "aliases": [
      "COL"
    ]
  },
  {
    "code": "CR",
    "isoCode": "es-CR",
    "nameHu": "Costa Rica",
    "nameEn": "Costa Rica",
    "nativeName": "Español",
    "flag": "🇨🇷",
    "aliases": [
      "CRI"
    ]
  },
  {
    "code": "CU",
    "isoCode": "es-CU",
    "nameHu": "Kuba",
    "nameEn": "Cuba",
    "nativeName": "Español",
    "flag": "🇨🇺",
    "aliases": [
      "CUB"
    ]
  },
  {
    "code": "CV",
    "isoCode": "pt-CV",
    "nameHu": "Zöld-foki Köztársaság",
    "nameEn": "Cape Verde",
    "nativeName": "Português / Kabuverdianu",
    "flag": "🇨🇻",
    "aliases": [
      "CPV"
    ]
  },
  {
    "code": "CW",
    "isoCode": "nl-CW",
    "nameHu": "Curaçao",
    "nameEn": "Curaçao",
    "nativeName": "Nederlands / Papiamento",
    "flag": "🇨🇼",
    "aliases": [
      "CUW"
    ]
  },
  {
    "code": "CX",
    "isoCode": "en-CX",
    "nameHu": "Karácsony-sziget",
    "nameEn": "Christmas Island",
    "nativeName": "English",
    "flag": "🇨🇽",
    "aliases": [
      "CXR"
    ]
  },
  {
    "code": "CY",
    "isoCode": "el-CY",
    "nameHu": "Ciprus",
    "nameEn": "Cyprus",
    "nativeName": "Ελληνικά / Türkçe",
    "flag": "🇨🇾",
    "aliases": [
      "CYP"
    ]
  },
  {
    "code": "CZ",
    "isoCode": "cs-CZ",
    "nameHu": "Csehország",
    "nameEn": "Czech Republic",
    "nativeName": "Čeština",
    "flag": "🇨🇿",
    "aliases": [
      "CZE"
    ]
  },
  {
    "code": "DE",
    "isoCode": "de-DE",
    "nameHu": "Németország",
    "nameEn": "Germany",
    "nativeName": "Deutsch",
    "flag": "🇩🇪",
    "aliases": [
      "GER",
      "DEU"
    ]
  },
  {
    "code": "DJ",
    "isoCode": "fr-DJ",
    "nameHu": "Dzsibuti",
    "nameEn": "Djibouti",
    "nativeName": "Français / العربية",
    "flag": "🇩🇯",
    "aliases": [
      "DJI"
    ]
  },
  {
    "code": "DK",
    "isoCode": "da-DK",
    "nameHu": "Dánia",
    "nameEn": "Denmark",
    "nativeName": "Dansk",
    "flag": "🇩🇰",
    "aliases": [
      "DNK"
    ]
  },
  {
    "code": "DM",
    "isoCode": "en-DM",
    "nameHu": "Dominikai Közösség",
    "nameEn": "Dominica",
    "nativeName": "English",
    "flag": "🇩🇲",
    "aliases": [
      "DMA"
    ]
  },
  {
    "code": "DO",
    "isoCode": "es-DO",
    "nameHu": "Dominikai Köztársaság",
    "nameEn": "Dominican Republic",
    "nativeName": "Español",
    "flag": "🇩🇴",
    "aliases": [
      "DOM"
    ]
  },
  {
    "code": "DZ",
    "isoCode": "ar-DZ",
    "nameHu": "Algéria",
    "nameEn": "Algeria",
    "nativeName": "العربية / Français",
    "flag": "🇩🇿",
    "aliases": [
      "DZA"
    ]
  },
  {
    "code": "EC",
    "isoCode": "es-EC",
    "nameHu": "Ecuador",
    "nameEn": "Ecuador",
    "nativeName": "Español",
    "flag": "🇪🇨",
    "aliases": [
      "ECU"
    ]
  },
  {
    "code": "EE",
    "isoCode": "et-EE",
    "nameHu": "Észtország",
    "nameEn": "Estonia",
    "nativeName": "Eesti",
    "flag": "🇪🇪",
    "aliases": [
      "EST"
    ]
  },
  {
    "code": "EG",
    "isoCode": "ar-EG",
    "nameHu": "Egyiptom",
    "nameEn": "Egypt",
    "nativeName": "العربية",
    "flag": "🇪🇬",
    "aliases": [
      "EGY"
    ]
  },
  {
    "code": "EH",
    "isoCode": "es-EH",
    "nameHu": "Nyugat-Szahara",
    "nameEn": "Western Sahara",
    "nativeName": "العربية / Español",
    "flag": "🇪🇭",
    "aliases": [
      "ESH"
    ]
  },
  {
    "code": "ER",
    "isoCode": "ti-ER",
    "nameHu": "Eritrea",
    "nameEn": "Eritrea",
    "nativeName": "ትግርኛ / العربية / English",
    "flag": "🇪🇷",
    "aliases": [
      "ERI"
    ]
  },
  {
    "code": "ES",
    "isoCode": "es-ES",
    "nameHu": "Spanyolország",
    "nameEn": "Spain",
    "nativeName": "Español",
    "flag": "🇪🇸",
    "aliases": [
      "ESP",
      "SPA"
    ]
  },
  {
    "code": "ET",
    "isoCode": "am-ET",
    "nameHu": "Etiópia",
    "nameEn": "Ethiopia",
    "nativeName": "አማርኛ",
    "flag": "🇪🇹",
    "aliases": [
      "ETH"
    ]
  },
  {
    "code": "FI",
    "isoCode": "fi-FI",
    "nameHu": "Finnország",
    "nameEn": "Finland",
    "nativeName": "Suomi / Svenska",
    "flag": "🇫🇮",
    "aliases": [
      "FIN"
    ]
  },
  {
    "code": "FJ",
    "isoCode": "en-FJ",
    "nameHu": "Fidzsi-szigetek",
    "nameEn": "Fiji",
    "nativeName": "English / Na Vosa Vakaviti",
    "flag": "🇫🇯",
    "aliases": [
      "FJI"
    ]
  },
  {
    "code": "FK",
    "isoCode": "en-FK",
    "nameHu": "Falkland-szigetek",
    "nameEn": "Falkland Islands",
    "nativeName": "English",
    "flag": "🇫🇰",
    "aliases": [
      "FLK"
    ]
  },
  {
    "code": "FM",
    "isoCode": "en-FM",
    "nameHu": "Mikronézia",
    "nameEn": "Micronesia",
    "nativeName": "English",
    "flag": "🇫🇲",
    "aliases": [
      "FSM"
    ]
  },
  {
    "code": "FO",
    "isoCode": "fo-FO",
    "nameHu": "Feröer-szigetek",
    "nameEn": "Faroe Islands",
    "nativeName": "Føroyskt / Dansk",
    "flag": "🇫🇴",
    "aliases": [
      "FRO"
    ]
  },
  {
    "code": "FR",
    "isoCode": "fr-FR",
    "nameHu": "Franciaország",
    "nameEn": "France",
    "nativeName": "Français",
    "flag": "🇫🇷",
    "aliases": [
      "FRA"
    ]
  },
  {
    "code": "GA",
    "isoCode": "fr-GA",
    "nameHu": "Gabon",
    "nameEn": "Gabon",
    "nativeName": "Français",
    "flag": "🇬🇦",
    "aliases": [
      "GAB"
    ]
  },
  {
    "code": "GB",
    "isoCode": "en-GB",
    "nameHu": "Egyesült Királyság",
    "nameEn": "United Kingdom",
    "nativeName": "English (UK)",
    "flag": "🇬🇧",
    "aliases": [
      "UK",
      "GBR",
      "GREAT BRITAIN"
    ]
  },
  {
    "code": "GD",
    "isoCode": "en-GD",
    "nameHu": "Grenada",
    "nameEn": "Grenada",
    "nativeName": "English",
    "flag": "🇬🇩",
    "aliases": [
      "GRD"
    ]
  },
  {
    "code": "GE",
    "isoCode": "ka-GE",
    "nameHu": "Grúzia",
    "nameEn": "Georgia",
    "nativeName": "ქართული",
    "flag": "🇬🇪",
    "aliases": [
      "GEO"
    ]
  },
  {
    "code": "GF",
    "isoCode": "fr-GF",
    "nameHu": "Francia Guyana",
    "nameEn": "French Guiana",
    "nativeName": "Français",
    "flag": "🇬🇫",
    "aliases": [
      "GUF"
    ]
  },
  {
    "code": "GG",
    "isoCode": "en-GG",
    "nameHu": "Guernsey",
    "nameEn": "Guernsey",
    "nativeName": "English",
    "flag": "🇬🇬",
    "aliases": [
      "GGY"
    ]
  },
  {
    "code": "GH",
    "isoCode": "en-GH",
    "nameHu": "Ghána",
    "nameEn": "Ghana",
    "nativeName": "English",
    "flag": "🇬🇭",
    "aliases": [
      "GHA"
    ]
  },
  {
    "code": "GI",
    "isoCode": "en-GI",
    "nameHu": "Gibraltár",
    "nameEn": "Gibraltar",
    "nativeName": "English",
    "flag": "🇬🇮",
    "aliases": [
      "GIB"
    ]
  },
  {
    "code": "GL",
    "isoCode": "kl-GL",
    "nameHu": "Grönland",
    "nameEn": "Greenland",
    "nativeName": "Kalaallisut / Dansk",
    "flag": "🇬🇱",
    "aliases": [
      "GRL"
    ]
  },
  {
    "code": "GM",
    "isoCode": "en-GM",
    "nameHu": "Gambia",
    "nameEn": "Gambia",
    "nativeName": "English",
    "flag": "🇬🇲",
    "aliases": [
      "GMB"
    ]
  },
  {
    "code": "GN",
    "isoCode": "fr-GN",
    "nameHu": "Guinea",
    "nameEn": "Guinea",
    "nativeName": "Français",
    "flag": "🇬🇳",
    "aliases": [
      "GIN"
    ]
  },
  {
    "code": "GP",
    "isoCode": "fr-GP",
    "nameHu": "Guadeloupe",
    "nameEn": "Guadeloupe",
    "nativeName": "Français",
    "flag": "🇬🇵",
    "aliases": [
      "GLP"
    ]
  },
  {
    "code": "GQ",
    "isoCode": "es-GQ",
    "nameHu": "Egyenlítői-Guinea",
    "nameEn": "Equatorial Guinea",
    "nativeName": "Español / Français",
    "flag": "🇬🇶",
    "aliases": [
      "GNQ"
    ]
  },
  {
    "code": "GR",
    "isoCode": "el-GR",
    "nameHu": "Görögország",
    "nameEn": "Greece",
    "nativeName": "Ελληνικά",
    "flag": "🇬🇷",
    "aliases": [
      "GRC",
      "GRE"
    ]
  },
  {
    "code": "GS",
    "isoCode": "en-GS",
    "nameHu": "Déli-Georgia",
    "nameEn": "South Georgia",
    "nativeName": "English",
    "flag": "🇬🇸",
    "aliases": [
      "SGS"
    ]
  },
  {
    "code": "GT",
    "isoCode": "es-GT",
    "nameHu": "Guatemala",
    "nameEn": "Guatemala",
    "nativeName": "Español",
    "flag": "🇬🇹",
    "aliases": [
      "GTM"
    ]
  },
  {
    "code": "GU",
    "isoCode": "en-GU",
    "nameHu": "Guam",
    "nameEn": "Guam",
    "nativeName": "English / Chamoru",
    "flag": "🇬🇺",
    "aliases": [
      "GUM"
    ]
  },
  {
    "code": "GW",
    "isoCode": "pt-GW",
    "nameHu": "Bissau-Guinea",
    "nameEn": "Guinea-Bissau",
    "nativeName": "Português",
    "flag": "🇬🇼",
    "aliases": [
      "GNB"
    ]
  },
  {
    "code": "GY",
    "isoCode": "en-GY",
    "nameHu": "Guyana",
    "nameEn": "Guyana",
    "nativeName": "English",
    "flag": "🇬🇾",
    "aliases": [
      "GUY"
    ]
  },
  {
    "code": "HK",
    "isoCode": "zh-HK",
    "nameHu": "Hongkong",
    "nameEn": "Hong Kong",
    "nativeName": "中文 (繁體) / English",
    "flag": "🇭🇰",
    "aliases": [
      "HKG"
    ]
  },
  {
    "code": "HM",
    "isoCode": "en-HM",
    "nameHu": "Heard-sziget és McDonald-szigetek",
    "nameEn": "Heard Island",
    "nativeName": "English",
    "flag": "🇭🇲",
    "aliases": [
      "HMD"
    ]
  },
  {
    "code": "HN",
    "isoCode": "es-HN",
    "nameHu": "Honduras",
    "nameEn": "Honduras",
    "nativeName": "Español",
    "flag": "🇭🇳",
    "aliases": [
      "HND"
    ]
  },
  {
    "code": "HR",
    "isoCode": "hr-HR",
    "nameHu": "Horvátország",
    "nameEn": "Croatia",
    "nativeName": "Hrvatski",
    "flag": "🇭🇷",
    "aliases": [
      "CRO"
    ]
  },
  {
    "code": "HT",
    "isoCode": "fr-HT",
    "nameHu": "Haiti",
    "nameEn": "Haiti",
    "nativeName": "Kreyòl / Français",
    "flag": "🇭🇹",
    "aliases": [
      "HTI"
    ]
  },
  {
    "code": "ID",
    "isoCode": "id-ID",
    "nameHu": "Indonézia",
    "nameEn": "Indonesia",
    "nativeName": "Bahasa Indonesia",
    "flag": "🇮🇩",
    "aliases": [
      "IDN"
    ]
  },
  {
    "code": "IE",
    "isoCode": "en-IE",
    "nameHu": "Írország",
    "nameEn": "Ireland",
    "nativeName": "English / Gaeilge",
    "flag": "🇮🇪",
    "aliases": [
      "IRL"
    ]
  },
  {
    "code": "IL",
    "isoCode": "he-IL",
    "nameHu": "Izrael",
    "nameEn": "Israel",
    "nativeName": "עברית / العربية",
    "flag": "🇮🇱",
    "aliases": [
      "ISR"
    ]
  },
  {
    "code": "IM",
    "isoCode": "en-IM",
    "nameHu": "Man-sziget",
    "nameEn": "Isle of Man",
    "nativeName": "English / Gaelg",
    "flag": "🇮🇲",
    "aliases": [
      "IMN"
    ]
  },
  {
    "code": "IN",
    "isoCode": "hi-IN",
    "nameHu": "India",
    "nameEn": "India",
    "nativeName": "हिन्दी / English",
    "flag": "🇮🇳",
    "aliases": [
      "IND"
    ]
  },
  {
    "code": "INT",
    "isoCode": "intl",
    "nameHu": "Nemzetközi",
    "nameEn": "International",
    "nativeName": "International",
    "flag": "🌐",
    "aliases": [
      "GLOBAL",
      "WORLD"
    ]
  },
  {
    "code": "IO",
    "isoCode": "en-IO",
    "nameHu": "Brit Indiai-óceáni Terület",
    "nameEn": "British Indian Ocean Territory",
    "nativeName": "English",
    "flag": "🇮🇴",
    "aliases": [
      "IOT"
    ]
  },
  {
    "code": "IQ",
    "isoCode": "ar-IQ",
    "nameHu": "Irak",
    "nameEn": "Iraq",
    "nativeName": "العربية / کوردی",
    "flag": "🇮🇶",
    "aliases": [
      "IRQ"
    ]
  },
  {
    "code": "IR",
    "isoCode": "fa-IR",
    "nameHu": "Irán",
    "nameEn": "Iran",
    "nativeName": "فارسی",
    "flag": "🇮🇷",
    "aliases": [
      "IRN"
    ]
  },
  {
    "code": "IS",
    "isoCode": "is-IS",
    "nameHu": "Izland",
    "nameEn": "Iceland",
    "nativeName": "Íslenska",
    "flag": "🇮🇸",
    "aliases": [
      "ISL"
    ]
  },
  {
    "code": "IT",
    "isoCode": "it-IT",
    "nameHu": "Olaszország",
    "nameEn": "Italy",
    "nativeName": "Italiano",
    "flag": "🇮🇹",
    "aliases": [
      "ITA"
    ]
  },
  {
    "code": "JE",
    "isoCode": "en-JE",
    "nameHu": "Jersey",
    "nameEn": "Jersey",
    "nativeName": "English / Jèrriais",
    "flag": "🇯🇪",
    "aliases": [
      "JEY"
    ]
  },
  {
    "code": "JM",
    "isoCode": "en-JM",
    "nameHu": "Jamaica",
    "nameEn": "Jamaica",
    "nativeName": "English",
    "flag": "🇯🇲",
    "aliases": [
      "JAM"
    ]
  },
  {
    "code": "JO",
    "isoCode": "ar-JO",
    "nameHu": "Jordánia",
    "nameEn": "Jordan",
    "nativeName": "العربية",
    "flag": "🇯🇴",
    "aliases": [
      "JOR"
    ]
  },
  {
    "code": "JP",
    "isoCode": "ja-JP",
    "nameHu": "Japán",
    "nameEn": "Japan",
    "nativeName": "日本語",
    "flag": "🇯🇵",
    "aliases": [
      "JPN"
    ]
  },
  {
    "code": "KE",
    "isoCode": "sw-KE",
    "nameHu": "Kenya",
    "nameEn": "Kenya",
    "nativeName": "Kiswahili / English",
    "flag": "🇰🇪",
    "aliases": [
      "KEN"
    ]
  },
  {
    "code": "KG",
    "isoCode": "ky-KG",
    "nameHu": "Kirgizisztán",
    "nameEn": "Kyrgyzstan",
    "nativeName": "Кыргызча / Русский",
    "flag": "🇰🇬",
    "aliases": [
      "KGZ"
    ]
  },
  {
    "code": "KH",
    "isoCode": "km-KH",
    "nameHu": "Kambodzsa",
    "nameEn": "Cambodia",
    "nativeName": "ភាសាខ្មែរ",
    "flag": "🇰🇭",
    "aliases": [
      "KHM"
    ]
  },
  {
    "code": "KI",
    "isoCode": "en-KI",
    "nameHu": "Kiribati",
    "nameEn": "Kiribati",
    "nativeName": "English / I-Kiribati",
    "flag": "🇰🇮",
    "aliases": [
      "KIR"
    ]
  },
  {
    "code": "KM",
    "isoCode": "ar-KM",
    "nameHu": "Comore-szigetek",
    "nameEn": "Comoros",
    "nativeName": "Shikomori / Français / العربية",
    "flag": "🇰🇲",
    "aliases": [
      "COM"
    ]
  },
  {
    "code": "KN",
    "isoCode": "en-KN",
    "nameHu": "Saint Kitts és Nevis",
    "nameEn": "Saint Kitts and Nevis",
    "nativeName": "English",
    "flag": "🇰🇳",
    "aliases": [
      "KNA"
    ]
  },
  {
    "code": "KP",
    "isoCode": "ko-KP",
    "nameHu": "Észak-Korea",
    "nameEn": "North Korea",
    "nativeName": "조선말",
    "flag": "🇰🇵",
    "aliases": [
      "PRK"
    ]
  },
  {
    "code": "KR",
    "isoCode": "ko-KR",
    "nameHu": "Dél-Korea",
    "nameEn": "South Korea",
    "nativeName": "한국어",
    "flag": "🇰🇷",
    "aliases": [
      "KOR"
    ]
  },
  {
    "code": "KW",
    "isoCode": "ar-KW",
    "nameHu": "Kuvait",
    "nameEn": "Kuwait",
    "nativeName": "العربية",
    "flag": "🇰🇼",
    "aliases": [
      "KWT"
    ]
  },
  {
    "code": "KY",
    "isoCode": "en-KY",
    "nameHu": "Kajmán-szigetek",
    "nameEn": "Cayman Islands",
    "nativeName": "English",
    "flag": "🇰🇾",
    "aliases": [
      "CYM"
    ]
  },
  {
    "code": "KZ",
    "isoCode": "kk-KZ",
    "nameHu": "Kazahsztán",
    "nameEn": "Kazakhstan",
    "nativeName": "Қазақша / Русский",
    "flag": "🇰🇿",
    "aliases": [
      "KAZ"
    ]
  },
  {
    "code": "LA",
    "isoCode": "lo-LA",
    "nameHu": "Laosz",
    "nameEn": "Laos",
    "nativeName": "ພາສາລາວ",
    "flag": "🇱🇦",
    "aliases": [
      "LAO"
    ]
  },
  {
    "code": "LB",
    "isoCode": "ar-LB",
    "nameHu": "Libanon",
    "nameEn": "Lebanon",
    "nativeName": "العربية / Français",
    "flag": "🇱🇧",
    "aliases": [
      "LBN"
    ]
  },
  {
    "code": "LC",
    "isoCode": "en-LC",
    "nameHu": "Saint Lucia",
    "nameEn": "Saint Lucia",
    "nativeName": "English",
    "flag": "🇱🇨",
    "aliases": [
      "LCA"
    ]
  },
  {
    "code": "LI",
    "isoCode": "de-LI",
    "nameHu": "Liechtenstein",
    "nameEn": "Liechtenstein",
    "nativeName": "Deutsch",
    "flag": "🇱🇮",
    "aliases": [
      "LIE"
    ]
  },
  {
    "code": "LK",
    "isoCode": "si-LK",
    "nameHu": "Srí Lanka",
    "nameEn": "Sri Lanka",
    "nativeName": "සිංහල / தமிழ்",
    "flag": "🇱🇰",
    "aliases": [
      "LKA"
    ]
  },
  {
    "code": "LR",
    "isoCode": "en-LR",
    "nameHu": "Libéria",
    "nameEn": "Liberia",
    "nativeName": "English",
    "flag": "🇱🇷",
    "aliases": [
      "LBR"
    ]
  },
  {
    "code": "LS",
    "isoCode": "en-LS",
    "nameHu": "Lesotho",
    "nameEn": "Lesotho",
    "nativeName": "Sesotho / English",
    "flag": "🇱🇸",
    "aliases": [
      "LSO"
    ]
  },
  {
    "code": "LT",
    "isoCode": "lt-LT",
    "nameHu": "Litvánia",
    "nameEn": "Lithuania",
    "nativeName": "Lietuvių",
    "flag": "🇱🇹",
    "aliases": [
      "LTU"
    ]
  },
  {
    "code": "LU",
    "isoCode": "lb-LU",
    "nameHu": "Luxemburg",
    "nameEn": "Luxembourg",
    "nativeName": "Lëtzebuergesch / Français / Deutsch",
    "flag": "🇱🇺",
    "aliases": [
      "LUX"
    ]
  },
  {
    "code": "LV",
    "isoCode": "lv-LV",
    "nameHu": "Lettország",
    "nameEn": "Latvia",
    "nativeName": "Latviešu",
    "flag": "🇱🇻",
    "aliases": [
      "LVA"
    ]
  },
  {
    "code": "LY",
    "isoCode": "ar-LY",
    "nameHu": "Líbia",
    "nameEn": "Libya",
    "nativeName": "العربية",
    "flag": "🇱🇾",
    "aliases": [
      "LBY"
    ]
  },
  {
    "code": "MA",
    "isoCode": "ar-MA",
    "nameHu": "Marokkó",
    "nameEn": "Morocco",
    "nativeName": "العربية / ⵜⴰⵎⴰⵣⵉⵖⵜ",
    "flag": "🇲🇦",
    "aliases": [
      "MAR"
    ]
  },
  {
    "code": "MC",
    "isoCode": "fr-MC",
    "nameHu": "Monaco",
    "nameEn": "Monaco",
    "nativeName": "Français",
    "flag": "🇲🇨",
    "aliases": [
      "MCO"
    ]
  },
  {
    "code": "MD",
    "isoCode": "ro-MD",
    "nameHu": "Moldova",
    "nameEn": "Moldova",
    "nativeName": "Română",
    "flag": "🇲🇩",
    "aliases": [
      "MDA"
    ]
  },
  {
    "code": "ME",
    "isoCode": "sr-ME",
    "nameHu": "Montenegró",
    "nameEn": "Montenegro",
    "nativeName": "Crnogorski / Српски",
    "flag": "🇲🇪",
    "aliases": [
      "MNE"
    ]
  },
  {
    "code": "MF",
    "isoCode": "fr-MF",
    "nameHu": "Saint-Martin",
    "nameEn": "Saint Martin",
    "nativeName": "Français",
    "flag": "🇲🇫",
    "aliases": [
      "MAF"
    ]
  },
  {
    "code": "MG",
    "isoCode": "mg-MG",
    "nameHu": "Madagaszkár",
    "nameEn": "Madagascar",
    "nativeName": "Malagasy / Français",
    "flag": "🇲🇬",
    "aliases": [
      "MDG"
    ]
  },
  {
    "code": "MH",
    "isoCode": "en-MH",
    "nameHu": "Marshall-szigetek",
    "nameEn": "Marshall Islands",
    "nativeName": "English / Kajin M̧ajeļ",
    "flag": "🇲🇭",
    "aliases": [
      "MHL"
    ]
  },
  {
    "code": "MK",
    "isoCode": "mk-MK",
    "nameHu": "Észak-Macedónia",
    "nameEn": "North Macedonia",
    "nativeName": "Македонски",
    "flag": "🇲🇰",
    "aliases": [
      "MKD"
    ]
  },
  {
    "code": "ML",
    "isoCode": "fr-ML",
    "nameHu": "Mali",
    "nameEn": "Mali",
    "nativeName": "Bamanankan / Français",
    "flag": "🇲🇱",
    "aliases": [
      "MLI"
    ]
  },
  {
    "code": "MM",
    "isoCode": "my-MM",
    "nameHu": "Mianmar",
    "nameEn": "Myanmar",
    "nativeName": "မြန်မာစာ",
    "flag": "🇲🇲",
    "aliases": [
      "MMR"
    ]
  },
  {
    "code": "MN",
    "isoCode": "mn-MN",
    "nameHu": "Mongólia",
    "nameEn": "Mongolia",
    "nativeName": "Монгол хэл",
    "flag": "🇲🇳",
    "aliases": [
      "MNG"
    ]
  },
  {
    "code": "MO",
    "isoCode": "zh-MO",
    "nameHu": "Makaó",
    "nameEn": "Macau",
    "nativeName": "中文 (繁體) / Português",
    "flag": "🇲🇴",
    "aliases": [
      "MAC"
    ]
  },
  {
    "code": "MP",
    "isoCode": "en-MP",
    "nameHu": "Északi-Mariana-szigetek",
    "nameEn": "Northern Mariana Islands",
    "nativeName": "English / Chamoru",
    "flag": "🇲🇵",
    "aliases": [
      "MNP"
    ]
  },
  {
    "code": "MQ",
    "isoCode": "fr-MQ",
    "nameHu": "Martinique",
    "nameEn": "Martinique",
    "nativeName": "Français",
    "flag": "🇲🇶",
    "aliases": [
      "MTQ"
    ]
  },
  {
    "code": "MR",
    "isoCode": "ar-MR",
    "nameHu": "Mauritánia",
    "nameEn": "Mauritania",
    "nativeName": "العربية / Français",
    "flag": "🇲🇷",
    "aliases": [
      "MRT"
    ]
  },
  {
    "code": "MS",
    "isoCode": "en-MS",
    "nameHu": "Montserrat",
    "nameEn": "Montserrat",
    "nativeName": "English",
    "flag": "🇲🇸",
    "aliases": [
      "MSR"
    ]
  },
  {
    "code": "MT",
    "isoCode": "mt-MT",
    "nameHu": "Málta",
    "nameEn": "Malta",
    "nativeName": "Malti / English",
    "flag": "🇲🇹",
    "aliases": [
      "MLT"
    ]
  },
  {
    "code": "MU",
    "isoCode": "en-MU",
    "nameHu": "Mauritius",
    "nameEn": "Mauritius",
    "nativeName": "English / Français",
    "flag": "🇲🇺",
    "aliases": [
      "MUS"
    ]
  },
  {
    "code": "MV",
    "isoCode": "dv-MV",
    "nameHu": "Maldív-szigetek",
    "nameEn": "Maldives",
    "nativeName": "ދިވެހިބަސް",
    "flag": "🇲🇻",
    "aliases": [
      "MDV"
    ]
  },
  {
    "code": "MW",
    "isoCode": "en-MW",
    "nameHu": "Malawi",
    "nameEn": "Malawi",
    "nativeName": "English / Chichewa",
    "flag": "🇲🇼",
    "aliases": [
      "MWI"
    ]
  },
  {
    "code": "MX",
    "isoCode": "es-MX",
    "nameHu": "Mexikó",
    "nameEn": "Mexico",
    "nativeName": "Español",
    "flag": "🇲🇽",
    "aliases": [
      "MEX"
    ]
  },
  {
    "code": "MY",
    "isoCode": "ms-MY",
    "nameHu": "Malajzia",
    "nameEn": "Malaysia",
    "nativeName": "Bahasa Melayu",
    "flag": "🇲🇾",
    "aliases": [
      "MYS"
    ]
  },
  {
    "code": "MZ",
    "isoCode": "pt-MZ",
    "nameHu": "Mozambik",
    "nameEn": "Mozambique",
    "nativeName": "Português",
    "flag": "🇲🇿",
    "aliases": [
      "MOZ"
    ]
  },
  {
    "code": "NA",
    "isoCode": "en-NA",
    "nameHu": "Namíbia",
    "nameEn": "Namibia",
    "nativeName": "English / Afrikaans / Deutsch",
    "flag": "🇳🇦",
    "aliases": [
      "NAM"
    ]
  },
  {
    "code": "NC",
    "isoCode": "fr-NC",
    "nameHu": "Új-Kaledónia",
    "nameEn": "New Caledonia",
    "nativeName": "Français",
    "flag": "🇳🇨",
    "aliases": [
      "NCL"
    ]
  },
  {
    "code": "NE",
    "isoCode": "fr-NE",
    "nameHu": "Niger",
    "nameEn": "Niger",
    "nativeName": "Français / Hausa",
    "flag": "🇳🇪",
    "aliases": [
      "NER"
    ]
  },
  {
    "code": "NF",
    "isoCode": "en-NF",
    "nameHu": "Norfolk-sziget",
    "nameEn": "Norfolk Island",
    "nativeName": "English / Norfuk",
    "flag": "🇳🇫",
    "aliases": [
      "NFK"
    ]
  },
  {
    "code": "NG",
    "isoCode": "en-NG",
    "nameHu": "Nigéria",
    "nameEn": "Nigeria",
    "nativeName": "English / Hausa / Yoruba / Igbo",
    "flag": "🇳🇬",
    "aliases": [
      "NGA"
    ]
  },
  {
    "code": "NI",
    "isoCode": "es-NI",
    "nameHu": "Nicaragua",
    "nameEn": "Nicaragua",
    "nativeName": "Español",
    "flag": "🇳🇮",
    "aliases": [
      "NIC"
    ]
  },
  {
    "code": "NL",
    "isoCode": "nl-NL",
    "nameHu": "Hollandia",
    "nameEn": "Netherlands",
    "nativeName": "Nederlands",
    "flag": "🇳🇱",
    "aliases": [
      "NLD",
      "NED"
    ]
  },
  {
    "code": "NO",
    "isoCode": "nb-NO",
    "nameHu": "Norvégia",
    "nameEn": "Norway",
    "nativeName": "Norsk (Bokmål / Nynorsk)",
    "flag": "🇳🇴",
    "aliases": [
      "NOR"
    ]
  },
  {
    "code": "NP",
    "isoCode": "ne-NP",
    "nameHu": "Nepál",
    "nameEn": "Nepal",
    "nativeName": "नेपाली",
    "flag": "🇳🇵",
    "aliases": [
      "NPL"
    ]
  },
  {
    "code": "NR",
    "isoCode": "na-NR",
    "nameHu": "Nauru",
    "nameEn": "Nauru",
    "nativeName": "Dorerin Naoero / English",
    "flag": "🇳🇷",
    "aliases": [
      "NRU"
    ]
  },
  {
    "code": "NU",
    "isoCode": "en-NU",
    "nameHu": "Niue",
    "nameEn": "Niue",
    "nativeName": "Ko e Vagahau Niuē / English",
    "flag": "🇳🇺",
    "aliases": [
      "NIU"
    ]
  },
  {
    "code": "NZ",
    "isoCode": "en-NZ",
    "nameHu": "Új-Zéland",
    "nameEn": "New Zealand",
    "nativeName": "English / Te Reo Māori",
    "flag": "🇳🇿",
    "aliases": [
      "NZL"
    ]
  },
  {
    "code": "OM",
    "isoCode": "ar-OM",
    "nameHu": "Omán",
    "nameEn": "Oman",
    "nativeName": "العربية",
    "flag": "🇴🇲",
    "aliases": [
      "OMN"
    ]
  },
  {
    "code": "PA",
    "isoCode": "es-PA",
    "nameHu": "Panama",
    "nameEn": "Panama",
    "nativeName": "Español",
    "flag": "🇵🇦",
    "aliases": [
      "PAN"
    ]
  },
  {
    "code": "PE",
    "isoCode": "es-PE",
    "nameHu": "Peru",
    "nameEn": "Peru",
    "nativeName": "Español / Quechua / Aymara",
    "flag": "🇵🇪",
    "aliases": [
      "PER"
    ]
  },
  {
    "code": "PF",
    "isoCode": "fr-PF",
    "nameHu": "Francia Polinézia",
    "nameEn": "French Polynesia",
    "nativeName": "Français / Reo Tahiti",
    "flag": "🇵🇫",
    "aliases": [
      "PYF"
    ]
  },
  {
    "code": "PG",
    "isoCode": "en-PG",
    "nameHu": "Pápua Új-Guinea",
    "nameEn": "Papua New Guinea",
    "nativeName": "English / Tok Pisin / Hiri Motu",
    "flag": "🇵🇬",
    "aliases": [
      "PNG"
    ]
  },
  {
    "code": "PH",
    "isoCode": "en-PH",
    "nameHu": "Fülöp-szigetek",
    "nameEn": "Philippines",
    "nativeName": "Filipino / English",
    "flag": "🇵🇭",
    "aliases": [
      "PHL"
    ]
  },
  {
    "code": "PK",
    "isoCode": "ur-PK",
    "nameHu": "Pakisztán",
    "nameEn": "Pakistan",
    "nativeName": "اردو / English",
    "flag": "🇵🇰",
    "aliases": [
      "PAK"
    ]
  },
  {
    "code": "PL",
    "isoCode": "pl-PL",
    "nameHu": "Lengyelország",
    "nameEn": "Poland",
    "nativeName": "Polski",
    "flag": "🇵🇱",
    "aliases": [
      "POL"
    ]
  },
  {
    "code": "PM",
    "isoCode": "fr-PM",
    "nameHu": "Saint-Pierre és Miquelon",
    "nameEn": "Saint Pierre and Miquelon",
    "nativeName": "Français",
    "flag": "🇵🇲",
    "aliases": [
      "SPM"
    ]
  },
  {
    "code": "PN",
    "isoCode": "en-PN",
    "nameHu": "Pitcairn-szigetek",
    "nameEn": "Pitcairn Islands",
    "nativeName": "English / Pitkern",
    "flag": "🇵🇳",
    "aliases": [
      "PCN"
    ]
  },
  {
    "code": "PR",
    "isoCode": "es-PR",
    "nameHu": "Puerto Rico",
    "nameEn": "Puerto Rico",
    "nativeName": "Español / English",
    "flag": "🇵🇷",
    "aliases": [
      "PRI"
    ]
  },
  {
    "code": "PS",
    "isoCode": "ar-PS",
    "nameHu": "Palesztina",
    "nameEn": "Palestine",
    "nativeName": "العربية",
    "flag": "🇵🇸",
    "aliases": [
      "PSE"
    ]
  },
  {
    "code": "PT",
    "isoCode": "pt-PT",
    "nameHu": "Portugália",
    "nameEn": "Portugal",
    "nativeName": "Português",
    "flag": "🇵🇹",
    "aliases": [
      "POR",
      "PRT"
    ]
  },
  {
    "code": "PW",
    "isoCode": "en-PW",
    "nameHu": "Palau",
    "nameEn": "Palau",
    "nativeName": "Belauan / English",
    "flag": "🇵🇼",
    "aliases": [
      "PLW"
    ]
  },
  {
    "code": "PY",
    "isoCode": "es-PY",
    "nameHu": "Paraguay",
    "nameEn": "Paraguay",
    "nativeName": "Español / Avañe'ẽ",
    "flag": "🇵🇾",
    "aliases": [
      "PRY"
    ]
  },
  {
    "code": "QA",
    "isoCode": "ar-QA",
    "nameHu": "Katar",
    "nameEn": "Qatar",
    "nativeName": "العربية",
    "flag": "🇶🇦",
    "aliases": [
      "QAT"
    ]
  },
  {
    "code": "RE",
    "isoCode": "fr-RE",
    "nameHu": "Réunion",
    "nameEn": "Réunion",
    "nativeName": "Français",
    "flag": "🇷🇪",
    "aliases": [
      "REU"
    ]
  },
  {
    "code": "RO",
    "isoCode": "ro-RO",
    "nameHu": "Románia",
    "nameEn": "Romania",
    "nativeName": "Română",
    "flag": "🇷🇴",
    "aliases": [
      "ROU",
      "ROM"
    ]
  },
  {
    "code": "RS",
    "isoCode": "sr-RS",
    "nameHu": "Szerbia",
    "nameEn": "Serbia",
    "nativeName": "Српски",
    "flag": "🇷🇸",
    "aliases": [
      "SRB"
    ]
  },
  {
    "code": "RU",
    "isoCode": "ru-RU",
    "nameHu": "Oroszország",
    "nameEn": "Russia",
    "nativeName": "Русский",
    "flag": "🇷🇺",
    "aliases": [
      "RUS"
    ]
  },
  {
    "code": "RW",
    "isoCode": "rw-RW",
    "nameHu": "Ruanda",
    "nameEn": "Rwanda",
    "nativeName": "Kinyarwanda / Français / English",
    "flag": "🇷🇼",
    "aliases": [
      "RWA"
    ]
  },
  {
    "code": "SA",
    "isoCode": "ar-SA",
    "nameHu": "Szaúd-Arábia",
    "nameEn": "Saudi Arabia",
    "nativeName": "العربية",
    "flag": "🇸🇦",
    "aliases": [
      "SAU"
    ]
  },
  {
    "code": "SB",
    "isoCode": "en-SB",
    "nameHu": "Salamon-szigetek",
    "nameEn": "Solomon Islands",
    "nativeName": "English",
    "flag": "🇸🇧",
    "aliases": [
      "SLB"
    ]
  },
  {
    "code": "SC",
    "isoCode": "fr-SC",
    "nameHu": "Seychelle-szigetek",
    "nameEn": "Seychelles",
    "nativeName": "Seychellois Creole / English / Français",
    "flag": "🇸🇨",
    "aliases": [
      "SYC"
    ]
  },
  {
    "code": "SD",
    "isoCode": "ar-SD",
    "nameHu": "Szudán",
    "nameEn": "Sudan",
    "nativeName": "العربية / English",
    "flag": "🇸🇩",
    "aliases": [
      "SDN"
    ]
  },
  {
    "code": "SE",
    "isoCode": "sv-SE",
    "nameHu": "Svédország",
    "nameEn": "Sweden",
    "nativeName": "Svenska",
    "flag": "🇸🇪",
    "aliases": [
      "SWE"
    ]
  },
  {
    "code": "SG",
    "isoCode": "en-SG",
    "nameHu": "Szingapúr",
    "nameEn": "Singapore",
    "nativeName": "English / Malay / Chinese / Tamil",
    "flag": "🇸🇬",
    "aliases": [
      "SGP"
    ]
  },
  {
    "code": "SH",
    "isoCode": "en-SH",
    "nameHu": "Szent Ilona",
    "nameEn": "Saint Helena",
    "nativeName": "English",
    "flag": "🇸🇭",
    "aliases": [
      "SHN"
    ]
  },
  {
    "code": "SI",
    "isoCode": "sl-SI",
    "nameHu": "Szlovénia",
    "nameEn": "Slovenia",
    "nativeName": "Slovenščina",
    "flag": "🇸🇮",
    "aliases": [
      "SVN"
    ]
  },
  {
    "code": "SJ",
    "isoCode": "no-SJ",
    "nameHu": "Svalbard és Jan Mayen",
    "nameEn": "Svalbard and Jan Mayen",
    "nativeName": "Norsk",
    "flag": "🇸🇯",
    "aliases": [
      "SJM"
    ]
  },
  {
    "code": "SK",
    "isoCode": "sk-SK",
    "nameHu": "Szlovákia",
    "nameEn": "Slovakia",
    "nativeName": "Slovenčina",
    "flag": "🇸🇰",
    "aliases": [
      "SVK"
    ]
  },
  {
    "code": "SL",
    "isoCode": "en-SL",
    "nameHu": "Sierra Leone",
    "nameEn": "Sierra Leone",
    "nativeName": "English",
    "flag": "🇸🇱",
    "aliases": [
      "SLE"
    ]
  },
  {
    "code": "SM",
    "isoCode": "it-SM",
    "nameHu": "San Marino",
    "nameEn": "San Marino",
    "nativeName": "Italiano",
    "flag": "🇸🇲",
    "aliases": [
      "SMR"
    ]
  },
  {
    "code": "SN",
    "isoCode": "fr-SN",
    "nameHu": "Szenegál",
    "nameEn": "Senegal",
    "nativeName": "Français / Wolof",
    "flag": "🇸🇳",
    "aliases": [
      "SEN"
    ]
  },
  {
    "code": "SO",
    "isoCode": "so-SO",
    "nameHu": "Szomália",
    "nameEn": "Somalia",
    "nativeName": "Soomaaliga / العربية",
    "flag": "🇸🇴",
    "aliases": [
      "SOM"
    ]
  },
  {
    "code": "SR",
    "isoCode": "nl-SR",
    "nameHu": "Suriname",
    "nameEn": "Suriname",
    "nativeName": "Nederlands",
    "flag": "🇸🇷",
    "aliases": [
      "SUR"
    ]
  },
  {
    "code": "SS",
    "isoCode": "en-SS",
    "nameHu": "Dél-Szudán",
    "nameEn": "South Sudan",
    "nativeName": "English",
    "flag": "🇸🇸",
    "aliases": [
      "SSD"
    ]
  },
  {
    "code": "ST",
    "isoCode": "pt-ST",
    "nameHu": "São Tomé és Príncipe",
    "nameEn": "São Tomé and Príncipe",
    "nativeName": "Português",
    "flag": "🇸🇹",
    "aliases": [
      "STP"
    ]
  },
  {
    "code": "SV",
    "isoCode": "es-SV",
    "nameHu": "Salvador",
    "nameEn": "El Salvador",
    "nativeName": "Español",
    "flag": "🇸🇻",
    "aliases": [
      "SLV"
    ]
  },
  {
    "code": "SX",
    "isoCode": "nl-SX",
    "nameHu": "Sint Maarten",
    "nameEn": "Sint Maarten",
    "nativeName": "Nederlands / English",
    "flag": "🇸🇽",
    "aliases": [
      "SXM"
    ]
  },
  {
    "code": "SY",
    "isoCode": "ar-SY",
    "nameHu": "Szíria",
    "nameEn": "Syria",
    "nativeName": "العربية",
    "flag": "🇸🇾",
    "aliases": [
      "SYR"
    ]
  },
  {
    "code": "SZ",
    "isoCode": "en-SZ",
    "nameHu": "Szváziföld (Eswatini)",
    "nameEn": "Eswatini",
    "nativeName": "siSwati / English",
    "flag": "🇸🇿",
    "aliases": [
      "SWZ"
    ]
  },
  {
    "code": "TC",
    "isoCode": "en-TC",
    "nameHu": "Turks- és Caicos-szigetek",
    "nameEn": "Turks and Caicos Islands",
    "nativeName": "English",
    "flag": "🇹🇨",
    "aliases": [
      "TCA"
    ]
  },
  {
    "code": "TD",
    "isoCode": "fr-TD",
    "nameHu": "Csád",
    "nameEn": "Chad",
    "nativeName": "Français / العربية",
    "flag": "🇹🇩",
    "aliases": [
      "TCD"
    ]
  },
  {
    "code": "TF",
    "isoCode": "fr-TF",
    "nameHu": "Francia Déli Területek",
    "nameEn": "French Southern Territories",
    "nativeName": "Français",
    "flag": "🇹🇫",
    "aliases": [
      "ATF"
    ]
  },
  {
    "code": "TG",
    "isoCode": "fr-TG",
    "nameHu": "Togo",
    "nameEn": "Togo",
    "nativeName": "Français",
    "flag": "🇹🇬",
    "aliases": [
      "TGO"
    ]
  },
  {
    "code": "TH",
    "isoCode": "th-TH",
    "nameHu": "Thaiföld",
    "nameEn": "Thailand",
    "nativeName": "ไทย",
    "flag": "🇹🇭",
    "aliases": [
      "THA"
    ]
  },
  {
    "code": "TJ",
    "isoCode": "tg-TJ",
    "nameHu": "Tádzsikisztán",
    "nameEn": "Tajikistan",
    "nativeName": "Тоҷикӣ",
    "flag": "🇹🇯",
    "aliases": [
      "TJK"
    ]
  },
  {
    "code": "TK",
    "isoCode": "en-TK",
    "nameHu": "Tokelau",
    "nameEn": "Tokelau",
    "nativeName": "Tokelauan / English",
    "flag": "🇹🇰",
    "aliases": [
      "TKL"
    ]
  },
  {
    "code": "TL",
    "isoCode": "pt-TL",
    "nameHu": "Kelet-Timor",
    "nameEn": "East Timor",
    "nativeName": "Tetum / Português",
    "flag": "🇹🇱",
    "aliases": [
      "TLS"
    ]
  },
  {
    "code": "TM",
    "isoCode": "tk-TM",
    "nameHu": "Türkmenisztán",
    "nameEn": "Turkmenistan",
    "nativeName": "Türkmençe",
    "flag": "🇹🇲",
    "aliases": [
      "TKM"
    ]
  },
  {
    "code": "TN",
    "isoCode": "ar-TN",
    "nameHu": "Tunézia",
    "nameEn": "Tunisia",
    "nativeName": "العربية / Français",
    "flag": "🇹🇳",
    "aliases": [
      "TUN"
    ]
  },
  {
    "code": "TO",
    "isoCode": "en-TO",
    "nameHu": "Tonga",
    "nameEn": "Tonga",
    "nativeName": "Lea Faka-Tonga / English",
    "flag": "🇹🇴",
    "aliases": [
      "TON"
    ]
  },
  {
    "code": "TR",
    "isoCode": "tr-TR",
    "nameHu": "Törökország",
    "nameEn": "Turkey",
    "nativeName": "Türkçe",
    "flag": "🇹🇷",
    "aliases": [
      "TUR"
    ]
  },
  {
    "code": "TT",
    "isoCode": "en-TT",
    "nameHu": "Trinidad és Tobago",
    "nameEn": "Trinidad and Tobago",
    "nativeName": "English",
    "flag": "🇹🇹",
    "aliases": [
      "TTO"
    ]
  },
  {
    "code": "TV",
    "isoCode": "en-TV",
    "nameHu": "Tuvalu",
    "nameEn": "Tuvalu",
    "nativeName": "Tuvaluan / English",
    "flag": "🇹🇻",
    "aliases": [
      "TUV"
    ]
  },
  {
    "code": "TW",
    "isoCode": "zh-TW",
    "nameHu": "Tajvan",
    "nameEn": "Taiwan",
    "nativeName": "中文 (繁體)",
    "flag": "🇹🇼",
    "aliases": [
      "TWN"
    ]
  },
  {
    "code": "TZ",
    "isoCode": "sw-TZ",
    "nameHu": "Tanzánia",
    "nameEn": "Tanzania",
    "nativeName": "Kiswahili / English",
    "flag": "🇹🇿",
    "aliases": [
      "TZA"
    ]
  },
  {
    "code": "UA",
    "isoCode": "uk-UA",
    "nameHu": "Ukrajna",
    "nameEn": "Ukraine",
    "nativeName": "Українська",
    "flag": "🇺🇦",
    "aliases": [
      "UKR"
    ]
  },
  {
    "code": "UG",
    "isoCode": "en-UG",
    "nameHu": "Uganda",
    "nameEn": "Uganda",
    "nativeName": "English / Luganda",
    "flag": "🇺🇬",
    "aliases": [
      "UGA"
    ]
  },
  {
    "code": "UM",
    "isoCode": "en-UM",
    "nameHu": "Amerikai Csendes-óceáni Szigetek",
    "nameEn": "US Minor Outlying Islands",
    "nativeName": "English",
    "flag": "🇺🇲",
    "aliases": [
      "UMI"
    ]
  },
  {
    "code": "US",
    "isoCode": "en-US",
    "nameHu": "Egyesült Államok",
    "nameEn": "United States",
    "nativeName": "English (US)",
    "flag": "🇺🇸",
    "aliases": [
      "USA",
      "UNITED STATES"
    ]
  },
  {
    "code": "UY",
    "isoCode": "es-UY",
    "nameHu": "Uruguay",
    "nameEn": "Uruguay",
    "nativeName": "Español",
    "flag": "🇺🇾",
    "aliases": [
      "URY"
    ]
  },
  {
    "code": "UZ",
    "isoCode": "uz-UZ",
    "nameHu": "Üzbegisztán",
    "nameEn": "Uzbekistan",
    "nativeName": "Oʻzbekcha",
    "flag": "🇺🇿",
    "aliases": [
      "UZB"
    ]
  },
  {
    "code": "VA",
    "isoCode": "it-VA",
    "nameHu": "Vatikán",
    "nameEn": "Vatican City",
    "nativeName": "Italiano / Latina",
    "flag": "🇻🇦",
    "aliases": [
      "VAT"
    ]
  },
  {
    "code": "VC",
    "isoCode": "en-VC",
    "nameHu": "Saint Vincent és a Grenadine-szigetek",
    "nameEn": "Saint Vincent and the Grenadines",
    "nativeName": "English",
    "flag": "🇻🇨",
    "aliases": [
      "VCT"
    ]
  },
  {
    "code": "VE",
    "isoCode": "es-VE",
    "nameHu": "Venezuela",
    "nameEn": "Venezuela",
    "nativeName": "Español",
    "flag": "🇻🇪",
    "aliases": [
      "VEN"
    ]
  },
  {
    "code": "VG",
    "isoCode": "en-VG",
    "nameHu": "Brit Virgin-szigetek",
    "nameEn": "British Virgin Islands",
    "nativeName": "English",
    "flag": "🇻🇬",
    "aliases": [
      "VGB"
    ]
  },
  {
    "code": "VI",
    "isoCode": "en-VI",
    "nameHu": "Amerikai Virgin-szigetek",
    "nameEn": "U.S. Virgin Islands",
    "nativeName": "English",
    "flag": "🇻🇮",
    "aliases": [
      "VIR"
    ]
  },
  {
    "code": "VN",
    "isoCode": "vi-VN",
    "nameHu": "Vietnam",
    "nameEn": "Vietnam",
    "nativeName": "Tiếng Việt",
    "flag": "🇻🇳",
    "aliases": [
      "VNM"
    ]
  },
  {
    "code": "VU",
    "isoCode": "bi-VU",
    "nameHu": "Vanuatu",
    "nameEn": "Vanuatu",
    "nativeName": "Bislama / English / Français",
    "flag": "🇻🇺",
    "aliases": [
      "VUT"
    ]
  },
  {
    "code": "WF",
    "isoCode": "fr-WF",
    "nameHu": "Wallis és Futuna",
    "nameEn": "Wallis and Futuna",
    "nativeName": "Français",
    "flag": "🇼🇫",
    "aliases": [
      "WLF"
    ]
  },
  {
    "code": "WS",
    "isoCode": "sm-WS",
    "nameHu": "Szamoa",
    "nameEn": "Samoa",
    "nativeName": "Gagana Sāmoa / English",
    "flag": "🇼🇸",
    "aliases": [
      "WSM"
    ]
  },
  {
    "code": "YE",
    "isoCode": "ar-YE",
    "nameHu": "Jemen",
    "nameEn": "Yemen",
    "nativeName": "العربية",
    "flag": "🇾🇪",
    "aliases": [
      "YEM"
    ]
  },
  {
    "code": "YT",
    "isoCode": "fr-YT",
    "nameHu": "Mayotte",
    "nameEn": "Mayotte",
    "nativeName": "Français",
    "flag": "🇾🇹",
    "aliases": [
      "MYT"
    ]
  },
  {
    "code": "ZA",
    "isoCode": "af-ZA",
    "nameHu": "Dél-afrikai Köztársaság",
    "nameEn": "South Africa",
    "nativeName": "English / Afrikaans / isiZulu",
    "flag": "🇿🇦",
    "aliases": [
      "ZAF"
    ]
  },
  {
    "code": "ZM",
    "isoCode": "en-ZM",
    "nameHu": "Zambia",
    "nameEn": "Zambia",
    "nativeName": "English",
    "flag": "🇿🇲",
    "aliases": [
      "ZMB"
    ]
  },
  {
    "code": "ZW",
    "isoCode": "en-ZW",
    "nameHu": "Zimbabwe",
    "nameEn": "Zimbabwe",
    "nativeName": "English / Shona / Ndebele",
    "flag": "🇿🇼",
    "aliases": [
      "ZWE"
    ]
  }
];

  const AppCountries = {
    getAll() {
      return COUNTRIES;
    },

    get(code) {
      if (!code) return null;
      const clean = String(code).trim().toUpperCase();
      return COUNTRIES.find(
        (c) => c.code === clean || (c.aliases && c.aliases.includes(clean))
      ) || null;
    },

    normalizeCode(code) {
      const found = this.get(code);
      return found ? found.code : (code ? String(code).trim().toUpperCase() : 'HU');
    },

    getIsoCode(code) {
      const found = this.get(code);
      return found ? found.isoCode : 'hu-HU';
    },

    populateSelect(selectOrId, options = {}) {
      const select = typeof selectOrId === 'string'
        ? document.getElementById(selectOrId)
        : selectOrId;
      if (!select) return;

      const format = options.format || 'source';
      const includeFlags = options.includeFlags !== false;
      const currentVal = options.selected || select.value || 'HU';
      const normalizedCurrent = this.normalizeCode(currentVal);

      select.innerHTML = '';

      COUNTRIES.forEach((c) => {
        const opt = document.createElement('option');
        opt.value = c.code;

        const flagPrefix = includeFlags && c.flag ? ${c.flag}  : '';

        if (format === 'search') {
          opt.textContent = ${flagPrefix} ();
        } else if (format === 'iso') {
          opt.textContent = ${flagPrefix} [] - ;
        } else {
          opt.textContent = ${flagPrefix} -  ();
        }

        if (c.code === normalizedCurrent) {
          opt.selected = true;
        }

        select.appendChild(opt);
      });

      const hasSelected = Array.from(select.options).some((o) => o.value === normalizedCurrent);
      if (!hasSelected && normalizedCurrent) {
        const customOpt = document.createElement('option');
        customOpt.value = normalizedCurrent;
        customOpt.textContent = normalizedCurrent;
        customOpt.selected = true;
        select.appendChild(customOpt);
      }
    }
  };

  window.Countries = AppCountries;
})();
