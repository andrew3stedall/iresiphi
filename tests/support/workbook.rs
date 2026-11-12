#[allow(dead_code)]
pub fn workbook_bytes(cell: &str, epoch1904: bool) -> Vec<u8> {
    use std::io::Write;
    let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let files=[("[Content_Types].xml",r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/></Types>"#.to_owned()),
 ("_rels/.rels",r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>"#.to_owned()),
 ("xl/workbook.xml",format!(r#"<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><workbookPr date1904="{}"/><sheets><sheet name="Review" sheetId="1" r:id="rId1"/></sheets></workbook>"#,if epoch1904{1}else{0})),
 ("xl/_rels/workbook.xml.rels",r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/></Relationships>"#.to_owned()),
 ("xl/worksheets/sheet1.xml",format!(r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><dimension ref="A1:C2"/><sheetData><row r="1"><c r="A1" t="inlineStr"><is><t>id</t></is></c><c r="B1" t="inlineStr"><is><t>amount</t></is></c><c r="C1" t="inlineStr"><is><t>supplier</t></is></c></row><row r="2"><c r="A2" t="inlineStr"><is><t>x</t></is></c>{cell}<c r="C2" t="inlineStr"><is><t>A</t></is></c></row></sheetData></worksheet>"#))];
    for (name, text) in files {
        writer
            .start_file(name, zip::write::SimpleFileOptions::default())
            .unwrap();
        writer.write_all(text.as_bytes()).unwrap();
    }
    writer.finish().unwrap().into_inner()
}

/// Writes the enrichment workbook used by the invoice acceptance fixture.
#[allow(dead_code)]
pub fn enrichment_workbook(path: &std::path::Path) {
    use std::io::{Read, Write};
    let bytes = workbook_bytes(r#"<c r="B2" t="inlineStr"><is><t>7.00</t></is></c>"#, false);
    let mut original = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let mut result = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
    for i in 0..original.len() {
        let mut entry = original.by_index(i).unwrap();
        let name = entry.name().unwrap().into_owned();
        let mut content = String::new();
        entry.read_to_string(&mut content).unwrap();
        if name == "xl/worksheets/sheet1.xml" {
            content = content
                .replace("A1:C2", "A1:C4")
                .replace("<t>x</t>", "<t>1</t>");
            content=content.replace("</sheetData>",r#"<row r="3"><c r="A3" t="inlineStr"><is><t>1</t></is></c><c r="B3" t="inlineStr"><is><t>8.00</t></is></c><c r="C3" t="inlineStr"><is><t>B</t></is></c></row><row r="4"><c r="A4" t="inlineStr"><is><t>2</t></is></c><c r="B4" t="inlineStr"><is><t>9.00</t></is></c><c r="C4" t="inlineStr"><is><t>A</t></is></c></row></sheetData>"#);
        }
        result
            .start_file(name, zip::write::SimpleFileOptions::default())
            .unwrap();
        result.write_all(content.as_bytes()).unwrap();
    }
    result.finish().unwrap();
}
