use native_api_1c::native_api_1c_core::interface::AddInWrapper;
use spreadsheet::SpreadSheet;

#[test]
fn register_extension_name_is_spreadsheet() {
    let mut addin = SpreadSheet::default();
    let utf16 = addin.register_extension_as();
    let name: String = String::from_utf16_lossy(
        &utf16
            .iter()
            .copied()
            .take_while(|&c| c != 0)
            .collect::<Vec<u16>>(),
    );
    assert_eq!(
        name, "SpreadSheet",
        "имя класса для Новый(\"AddIn.SS.<имя>\")"
    );
}
