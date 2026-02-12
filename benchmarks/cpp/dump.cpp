// Open_AlteryxYXDB Dump Tool
//
// Reads a YXDB file using the Open_AlteryxYXDB (NedHarding C++) library
// and dumps all field values as tab-separated text to stdout.
// Null values are represented as "\N".
//
// This is used by the cross-implementation round-trip test to verify
// that SigilYX produces files readable by other YXDB implementations,
// and vice versa.
//
// Usage:
//     open_yxdb_dump.exe <path_to_yxdb>
//
// Output format:
//   - First line: tab-separated field names
//   - Second line: tab-separated field types (Bool, Int32, etc.)
//   - Remaining lines: tab-separated field values, one row per line
//   - Null values are represented as \N
//   - Blob values are represented as [blob:<length>]

#ifndef _CRT_SECURE_NO_WARNINGS
#define _CRT_SECURE_NO_WARNINGS
#endif
#ifndef _SCL_SECURE_NO_DEPRECATE
#define _SCL_SECURE_NO_DEPRECATE
#endif
#ifndef UNICODE
#define UNICODE
#endif
#define _LARGEFILE_SOURCE
#define _LARGEFILE64_SOURCE

#include "Open_AlteryxYXDB/stdafx.h"
#include "Open_AlteryxYXDB/SrcLib_Replacement.h"
#include "Open_AlteryxYXDB/lzf_src.h"
#include "Open_AlteryxYXDB/RecordLib/Record.h"
#include "Open_AlteryxYXDB/Open_AlteryxYXDB.h"

#include <cstdio>
#include <cstdlib>
#include <string>
#include <clocale>

#ifdef _WIN32
#include <io.h>
#include <fcntl.h>
#endif

// Convert wide string to UTF-8
std::string wstring_to_utf8(const wchar_t* wstr, size_t len) {
    if (len == 0) return "";
    std::string result;
    result.reserve(len);
    for (size_t i = 0; i < len; ++i) {
        wchar_t c = wstr[i];
        if (c < 0x80) {
            result += (char)c;
        } else if (c < 0x800) {
            result += (char)(0xC0 | (c >> 6));
            result += (char)(0x80 | (c & 0x3F));
        } else {
            result += (char)(0xE0 | (c >> 12));
            result += (char)(0x80 | ((c >> 6) & 0x3F));
            result += (char)(0x80 | (c & 0x3F));
        }
    }
    return result;
}

std::string wstring_to_utf8(const SRC::WString& ws) {
    return wstring_to_utf8(ws.c_str(), ws.Length());
}

// Get field type name as string
const char* field_type_name(SRC::E_FieldType ft) {
    switch (ft) {
        case SRC::E_FT_Bool: return "Bool";
        case SRC::E_FT_Byte: return "Byte";
        case SRC::E_FT_Int16: return "Int16";
        case SRC::E_FT_Int32: return "Int32";
        case SRC::E_FT_Int64: return "Int64";
        case SRC::E_FT_FixedDecimal: return "FixedDecimal";
        case SRC::E_FT_Float: return "Float";
        case SRC::E_FT_Double: return "Double";
        case SRC::E_FT_String: return "String";
        case SRC::E_FT_WString: return "WString";
        case SRC::E_FT_V_String: return "V_String";
        case SRC::E_FT_V_WString: return "V_WString";
        case SRC::E_FT_Date: return "Date";
        case SRC::E_FT_Time: return "Time";
        case SRC::E_FT_DateTime: return "DateTime";
        case SRC::E_FT_Blob: return "Blob";
        case SRC::E_FT_SpatialObj: return "SpatialObj";
        default: return "Unknown";
    }
}

// Convert narrow string to wide string
std::wstring to_wstring(const char* s) {
    std::wstring ws;
    while (*s) ws += (wchar_t)*s++;
    return ws;
}

// Escape a string for TSV output (replace tabs and newlines)
std::string escape_tsv(const std::string& s) {
    std::string result;
    result.reserve(s.size());
    for (char c : s) {
        if (c == '\t') result += "\\t";
        else if (c == '\n') result += "\\n";
        else if (c == '\r') result += "\\r";
        else if (c == '\\') result += "\\\\";
        else result += c;
    }
    return result;
}

int main(int argc, char* argv[]) {
    if (argc < 2) {
        fprintf(stderr, "Usage: %s <path_to_yxdb>\n", argv[0]);
        fprintf(stderr, "Dumps all records as TSV to stdout.\n");
        return 1;
    }

#ifdef _WIN32
    // Set stdout to binary mode to avoid \r\n translation
    _setmode(_fileno(stdout), _O_BINARY);
#endif

    const char* file_path_narrow = argv[1];
    std::wstring file_path = to_wstring(file_path_narrow);

    try {
        Alteryx::OpenYXDB::Open_AlteryxYXDB file;
        file.Open(file_path.c_str());

        unsigned num_fields = file.m_recordInfo.NumFields();

        // Output header: field names
        for (unsigned i = 0; i < num_fields; ++i) {
            if (i > 0) putchar('\t');
            const SRC::FieldBase* pField = file.m_recordInfo[i];
            std::string name = wstring_to_utf8(pField->GetFieldName());
            fputs(escape_tsv(name).c_str(), stdout);
        }
        putchar('\n');

        // Output field types
        for (unsigned i = 0; i < num_fields; ++i) {
            if (i > 0) putchar('\t');
            const SRC::FieldBase* pField = file.m_recordInfo[i];
            fputs(field_type_name(pField->m_ft), stdout);
        }
        putchar('\n');

        // Output records
        long long num_rows = 0;
        while (const SRC::RecordData* pRec = file.ReadRecord()) {
            for (unsigned i = 0; i < num_fields; ++i) {
                if (i > 0) putchar('\t');
                const SRC::FieldBase* pField = file.m_recordInfo[i];

                switch (pField->m_ft) {
                    case SRC::E_FT_Bool: {
                        auto val = pField->GetAsBool(pRec);
                        if (val.bIsNull)
                            fputs("\\N", stdout);
                        else
                            fputs(val.value ? "true" : "false", stdout);
                        break;
                    }
                    case SRC::E_FT_Byte:
                    case SRC::E_FT_Int16:
                    case SRC::E_FT_Int32: {
                        auto val = pField->GetAsInt32(pRec);
                        if (val.bIsNull)
                            fputs("\\N", stdout);
                        else
                            fprintf(stdout, "%d", val.value);
                        break;
                    }
                    case SRC::E_FT_Int64: {
                        auto val = pField->GetAsInt64(pRec);
                        if (val.bIsNull)
                            fputs("\\N", stdout);
                        else
                            fprintf(stdout, "%lld", val.value);
                        break;
                    }
                    case SRC::E_FT_Float:
                    case SRC::E_FT_Double: {
                        auto val = pField->GetAsDouble(pRec);
                        if (val.bIsNull)
                            fputs("\\N", stdout);
                        else
                            fprintf(stdout, "%.17g", val.value);
                        break;
                    }
                    case SRC::E_FT_FixedDecimal: {
                        // FixedDecimal: extract as string to preserve precision
                        auto val = pField->GetAsAString(pRec);
                        if (val.bIsNull)
                            fputs("\\N", stdout);
                        else {
                            std::string s(val.value.pValue, val.value.nLength);
                            fputs(escape_tsv(s).c_str(), stdout);
                        }
                        break;
                    }
                    case SRC::E_FT_String:
                    case SRC::E_FT_V_String: {
                        auto val = pField->GetAsAString(pRec);
                        if (val.bIsNull)
                            fputs("\\N", stdout);
                        else {
                            std::string s(val.value.pValue, val.value.nLength);
                            fputs(escape_tsv(s).c_str(), stdout);
                        }
                        break;
                    }
                    case SRC::E_FT_WString:
                    case SRC::E_FT_V_WString: {
                        auto val = pField->GetAsWString(pRec);
                        if (val.bIsNull)
                            fputs("\\N", stdout);
                        else {
                            std::string s = wstring_to_utf8(val.value.pValue, val.value.nLength);
                            fputs(escape_tsv(s).c_str(), stdout);
                        }
                        break;
                    }
                    case SRC::E_FT_Date:
                    case SRC::E_FT_Time:
                    case SRC::E_FT_DateTime: {
                        auto val = pField->GetAsWString(pRec);
                        if (val.bIsNull)
                            fputs("\\N", stdout);
                        else {
                            std::string s = wstring_to_utf8(val.value.pValue, val.value.nLength);
                            fputs(escape_tsv(s).c_str(), stdout);
                        }
                        break;
                    }
                    case SRC::E_FT_Blob:
                    case SRC::E_FT_SpatialObj: {
                        auto val = pField->GetAsBlob(pRec);
                        if (val.bIsNull)
                            fputs("\\N", stdout);
                        else
                            fprintf(stdout, "[blob:%u]", val.value.nLength);
                        break;
                    }
                    default: {
                        auto val = pField->GetAsWString(pRec);
                        if (val.bIsNull)
                            fputs("\\N", stdout);
                        else {
                            std::string s = wstring_to_utf8(val.value.pValue, val.value.nLength);
                            fputs(escape_tsv(s).c_str(), stdout);
                        }
                        break;
                    }
                }
            }
            putchar('\n');
            num_rows++;
        }

        file.Close();
        fprintf(stderr, "Dumped %lld records, %u fields\n", num_rows, num_fields);

    } catch (const SRC::Error& e) {
        fprintf(stderr, "ERROR: %s\n",
                SRC::ConvertToAString(e.GetErrorDescription()).c_str());
        return 2;
    } catch (const std::exception& e) {
        fprintf(stderr, "ERROR: %s\n", e.what());
        return 2;
    } catch (...) {
        fprintf(stderr, "ERROR: Unknown exception\n");
        return 2;
    }

    return 0;
}
