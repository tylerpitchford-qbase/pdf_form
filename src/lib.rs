//! This crate is for filling out PDFs with forms programatically.
extern crate lopdf;
#[macro_use]
extern crate bitflags;
#[macro_use]
extern crate derive_error;

use lopdf::{Document, Object, ObjectId, StringFormat};
use std::collections::VecDeque;
use std::io;
use std::io::Write;
use std::path::Path;
use std::str;

bitflags! {
    struct ButtonFlags: u32 {
        const NO_TOGGLE_TO_OFF  = 0x8000;
        const RADIO             = 0x10000;
        const PUSHBUTTON        = 0x20000;
        const RADIO_IN_UNISON   = 0x4000000;

    }
}

bitflags! {
    struct ChoiceFlags: u32 {
        const COBMO             = 0x20000;
        const EDIT              = 0x40000;
        const SORT              = 0x80000;
        const MULTISELECT       = 0x200000;
        const DO_NOT_SPELLCHECK = 0x800000;
        const COMMIT_ON_CHANGE  = 0x8000000;
    }
}

/// A PDF Form that contains fillable fields
///
/// Use this struct to load an existing PDF with a fillable form using the `load` method.  It will
/// analyze the PDF and identify the fields. Then you can get and set the content of the fields by
/// index.
pub struct Form {
    doc: Document,
    form_ids: Vec<ObjectId>,
}

/// The possible types of fillable form fields in a PDF
#[derive(Debug)]
pub enum FieldType {
    Button,
    Radio,
    CheckBox,
    ListBox,
    ComboBox,
    Text,
}

/// The current state of a form field
#[derive(Debug)]
pub enum FieldState {
    /// Push buttons have no state
    Button,
    /// `selected` is the sigular option from `options` that is selected
    Radio {
        selected: String,
        options: Vec<String>,
    },
    /// The toggle state of the checkbox
    CheckBox { is_checked: bool },
    /// `selected` is the list of selected options from `options`
    ListBox {
        selected: Vec<String>,
        options: Vec<String>,
        multiselect: bool,
    },
    /// `selected` is the list of selected options from `options`
    ComboBox {
        selected: Vec<String>,
        options: Vec<String>,
        editable: bool,
    },
    /// User Text Input
    Text { text: String },
}

#[derive(Debug, Error)]
/// Errors that may occur while loading a PDF
pub enum LoadError {
    /// An IO Error
    IoError(io::Error),
    /// A dictionary key that must be present in order to find forms was not present
    DictionaryKeyNotFound,
    /// The reference `ObjectId` did not point to any values
    #[error(non_std, no_from)]
    NoSuchReference(ObjectId),
    /// An element that was expected to be a reference was not a reference
    NotAReference,
    /// A value that must be a certain type was not that type
    UnexpectedType,
}

/// Errors That may occur while setting values in a form
#[derive(Debug, Error)]
pub enum ValueError {
    /// The method used to set the state is incompatible with the type of the field
    TypeMismatch,
    /// One or more selected values are not valid choices
    InvalidSelection,
    /// Multiple values were selected when only one was allowed
    TooManySelected,
}

trait PdfObjectDeref {
    fn deref<'a>(&self, doc: &'a Document) -> Result<&'a Object, LoadError>;
}

impl PdfObjectDeref for Object {
    fn deref<'a>(&self, doc: &'a Document) -> Result<&'a Object, LoadError> {
        match self {
            &Object::Reference(oid) => doc.objects.get(&oid).ok_or(LoadError::NoSuchReference(oid)),
            _ => Err(LoadError::NotAReference),
        }
    }
}

impl Form {
    /// Takes a reader containing a PDF with a fillable form, analyzes the content, and attempts to
    /// identify all of the fields the form has.
    pub fn load_from<R: io::Read>(reader: R) -> Result<Self, LoadError> {
        let doc = Document::load_from(reader).unwrap();
        Self::load_doc(doc)
    }

    /// Takes a path to a PDF with a fillable form, analyzes the file, and attempts to identify all
    /// of the fields the form has.
    pub fn load<P: AsRef<Path>>(path: P) -> Result<Self, LoadError> {
        let doc = Document::load(path).unwrap();
        Self::load_doc(doc)
    }

    fn load_doc(doc: Document) -> Result<Self, LoadError> {
        let mut form_ids = Vec::new();
        let mut queue = VecDeque::new();
        // Block so borrow of doc ends before doc is moved into the result
        {
            // Get the form's top level fields
            let catalog = doc
                .trailer
                .get(b"Root")
                .deref(&doc)?
                .as_dict()
                .ok_or(LoadError::UnexpectedType)?;
            let acroform = catalog
                .get(b"AcroForm")
                .ok_or(LoadError::DictionaryKeyNotFound)?
                .deref(&doc)?
                .as_dict()
                .ok_or(LoadError::UnexpectedType)?;
            let fields_list = acroform
                .get(b"Fields")
                .ok_or(LoadError::DictionaryKeyNotFound)?
                //    .deref(&doc)?
                .as_array()
                .ok_or(LoadError::UnexpectedType)?;
            queue.append(&mut VecDeque::from(fields_list.clone()));

            // Iterate over the fields
            while let Some(objref) = queue.pop_front() {
                let obj = objref.deref(&doc)?;
                if let &Object::Dictionary(ref dict) = obj {
                    // If the field has FT, it actually takes input.  Save this
                    if let Some(_) = dict.get(b"FT") {
                        form_ids.push(objref.as_reference().unwrap());
                    }
                    // If this field has kids, they might have FT, so add them to the queue
                    if let Some(&Object::Array(ref kids)) = dict.get(b"Kids") {
                        queue.append(&mut VecDeque::from(kids.clone()));
                    }
                }
            }
        }
        Ok(Form { doc, form_ids })
    }

    /// Returns the number of fields the form has
    pub fn len(&self) -> usize {
        self.form_ids.len()
    }

    /// Gets the type of field of the given index
    ///
    /// # Panics
    /// This function will panic if the index is greater than the number of fields
    pub fn get_type(&self, n: usize) -> FieldType {
        // unwraps should be fine because load should have verified everything exists
        let field = self
            .doc
            .objects
            .get(&self.form_ids[n])
            .unwrap()
            .as_dict()
            .unwrap();
        let obj_zero = Object::Integer(0);
        let type_str = field.get(b"FT").unwrap().as_name_str().unwrap();
        if type_str == "Btn" {
            let flags = ButtonFlags::from_bits_truncate(
                field.get(b"Ff").unwrap_or(&obj_zero).as_i64().unwrap() as u32,
            );
            if flags.intersects(ButtonFlags::RADIO | ButtonFlags::NO_TOGGLE_TO_OFF) {
                FieldType::Radio
            } else if flags.intersects(ButtonFlags::PUSHBUTTON) {
                FieldType::Button
            } else {
                FieldType::CheckBox
            }
        } else if type_str == "Ch" {
            let flags = ChoiceFlags::from_bits_truncate(
                field.get(b"Ff").unwrap_or(&obj_zero).as_i64().unwrap() as u32,
            );
            if flags.intersects(ChoiceFlags::COBMO) {
                FieldType::ComboBox
            } else {
                FieldType::ListBox
            }
        } else {
            FieldType::Text
        }
    }

    /// Gets the name of field of the given index
    ///
    /// # Panics
    /// This function will panic if the index is greater than the number of fields
    pub fn get_name(&self, n: usize) -> Option<String> {
        // unwraps should be fine because load should have verified everything exists
        let field = self
            .doc
            .objects
            .get(&self.form_ids[n])
            .unwrap()
            .as_dict()
            .unwrap();

        // The "T" key refers to the name of the field
        match field.get(b"T") {
            Some(Object::String(data, _)) => String::from_utf8(data.clone()).ok(),
            _ => None,
        }
    }

    /// Gets the types of all of the fields in the form
    pub fn get_all_types(&self) -> Vec<FieldType> {
        let mut res = Vec::with_capacity(self.len());
        for i in 0..self.len() {
            res.push(self.get_type(i))
        }
        res
    }

    /// Gets the names of all of the fields in the form
    pub fn get_all_names(&self) -> Vec<Option<String>> {
        let mut res = Vec::with_capacity(self.len());
        for i in 0..self.len() {
            res.push(self.get_name(i))
        }
        res
    }

    /// Gets the state of field of the given index
    ///
    /// # Panics
    /// This function will panic if the index is greater than the number of fields
    pub fn get_state(&self, n: usize) -> FieldState {
        let field = self
            .doc
            .objects
            .get(&self.form_ids[n])
            .unwrap()
            .as_dict()
            .unwrap();
        match self.get_type(n) {
            FieldType::Button => FieldState::Button,
            FieldType::Radio => FieldState::Radio {
                selected: match field.get(b"V") {
                    Some(name) => name.as_name_str().unwrap().to_owned(),
                    None => match field.get(b"AS") {
                        Some(name) => name.as_name_str().unwrap().to_owned(),
                        None => "".to_owned(),
                    },
                },
                options: self.get_possibilities(self.form_ids[n]),
            },
            FieldType::CheckBox => FieldState::CheckBox {
                is_checked: match field.get(b"V") {
                    Some(name) => {
                        if name.as_name_str().unwrap() == "Yes" {
                            true
                        } else {
                            false
                        }
                    }
                    None => match field.get(b"AS") {
                        Some(name) => {
                            if name.as_name_str().unwrap() == "Yes" {
                                true
                            } else {
                                false
                            }
                        }
                        None => false,
                    },
                },
            },
            FieldType::ListBox => FieldState::ListBox {
                // V field in a list box can be either text for one option, an array for many
                // options, or null
                selected: match field.get(b"V") {
                    Some(selection) => match selection {
                        &Object::String(ref s, StringFormat::Literal) => {
                            vec![str::from_utf8(&s).unwrap().to_owned()]
                        }
                        &Object::Array(ref chosen) => {
                            let mut res = Vec::new();
                            for obj in chosen {
                                if let &Object::String(ref s, StringFormat::Literal) = obj {
                                    res.push(str::from_utf8(&s).unwrap().to_owned());
                                }
                            }
                            res
                        }
                        _ => Vec::new(),
                    },
                    None => Vec::new(),
                },
                // The options is an array of either text elements or arrays where the second
                // element is what we want
                options: match field.get(b"Opt") {
                    Some(&Object::Array(ref options)) => options
                        .iter()
                        .map(|x| match x {
                            &Object::String(ref s, StringFormat::Literal) => {
                                str::from_utf8(&s).unwrap().to_owned()
                            }
                            &Object::Array(ref arr) => {
                                if let &Object::String(ref s, StringFormat::Literal) = &arr[1] {
                                    str::from_utf8(&s).unwrap().to_owned()
                                } else {
                                    String::new()
                                }
                            }
                            _ => String::new(),
                        })
                        .filter(|x| x.len() > 0)
                        .collect(),
                    _ => Vec::new(),
                },
                multiselect: {
                    let flags = ChoiceFlags::from_bits_truncate(
                        field.get(b"Ff").unwrap_or(&Object::Integer(0)).as_i64().unwrap() as u32,
                    );
                    flags.intersects(ChoiceFlags::MULTISELECT)
                },
            },
            FieldType::ComboBox => FieldState::ComboBox {
                // V field in a list box can be either text for one option, an array for many
                // options, or null
                selected: match field.get(b"V") {
                    Some(selection) => match selection {
                        &Object::String(ref s, StringFormat::Literal) => {
                            vec![str::from_utf8(&s).unwrap().to_owned()]
                        }
                        &Object::Array(ref chosen) => {
                            let mut res = Vec::new();
                            for obj in chosen {
                                if let &Object::String(ref s, StringFormat::Literal) = obj {
                                    res.push(str::from_utf8(&s).unwrap().to_owned());
                                }
                            }
                            res
                        }
                        _ => Vec::new(),
                    },
                    None => Vec::new(),
                },
                // The options is an array of either text elements or arrays where the second
                // element is what we want
                options: match field.get(b"Opt") {
                    Some(&Object::Array(ref options)) => options
                        .iter()
                        .map(|x| match x {
                            &Object::String(ref s, StringFormat::Literal) => {
                                str::from_utf8(&s).unwrap().to_owned()
                            }
                            &Object::Array(ref arr) => {
                                if let &Object::String(ref s, StringFormat::Literal) = &arr[1] {
                                    str::from_utf8(&s).unwrap().to_owned()
                                } else {
                                    String::new()
                                }
                            }
                            _ => String::new(),
                        })
                        .filter(|x| x.len() > 0)
                        .collect(),
                    _ => Vec::new(),
                },
                editable: {
                    let flags = ChoiceFlags::from_bits_truncate(
                        field.get(b"Ff").unwrap_or(&Object::Integer(0)).as_i64().unwrap() as u32,
                    );
                    flags.intersects(ChoiceFlags::EDIT)
                },
            },
            FieldType::Text => FieldState::Text {
                text: match field.get(b"V") {
                    Some(&Object::String(ref s, StringFormat::Literal)) => {
                        str::from_utf8(&s.clone()).unwrap().to_owned()
                    }
                    _ => "".to_owned(),
                },
            },
        }
    }

    /// If the field at index `n` is a text field, fills in that field with the text `s`.
    /// If it is not a text field, returns ValueError
    ///
    /// # Panics
    /// Will panic if n is larger than the number of fields
    pub fn set_text(&mut self, n: usize, s: String) -> Result<(), ValueError> {
        match self.get_type(n) {
            FieldType::Text => {
                let field = self
                    .doc
                    .objects
                    .get_mut(&self.form_ids[n])
                    .unwrap()
                    .as_dict_mut()
                    .unwrap();
                field.set(b"V", Object::String(s.into_bytes(), StringFormat::Literal));
                field.remove(b"AP");
                Ok(())
            }
            _ => Err(ValueError::TypeMismatch),
        }
    }

    fn get_possibilities(&self, oid: ObjectId) -> Vec<String> {
        let mut res = Vec::new();
        let kids_obj = self
            .doc
            .objects
            .get(&oid)
            .unwrap()
            .as_dict()
            .unwrap()
            .get(b"Kids");
        if let Some(&Object::Array(ref kids)) = kids_obj {
            for (i, kid) in kids.iter().enumerate() {
                let mut found = false;
                if let Some(&Object::Dictionary(ref appearance_states)) =
                    kid.deref(&self.doc).unwrap().as_dict().unwrap().get(b"AP")
                {
                    if let Some(&Object::Dictionary(ref normal_appearance)) =
                        appearance_states.get(b"N")
                    {
                        for (key, _) in normal_appearance {
                            if (key != "Off") {
                                res.push(key.to_owned());
                                found = true;
                                break;
                            }
                        }
                    }
                }
                if !found {
                    res.push(i.to_string());
                }
            }
        }
        res
    }

    /// If the field at index `n` is a checkbox field, toggles the check box based on the value
    /// `is_checked`.
    /// If it is not a checkbox field, returns ValueError
    ///
    /// # Panics
    /// Will panic if n is larger than the number of fields
    pub fn set_check_box(&mut self, n: usize, is_checked: bool) -> Result<(), ValueError> {
        match self.get_type(n) {
            FieldType::CheckBox => {
                let state = Object::Name(
                    {
                        if is_checked {
                            "Yes"
                        } else {
                            "Off"
                        }
                    }
                    .to_owned()
                    .into_bytes(),
                );
                let field = self
                    .doc
                    .objects
                    .get_mut(&self.form_ids[n])
                    .unwrap()
                    .as_dict_mut()
                    .unwrap();
                field.set(b"V", state.clone());
                field.set(b"AS", state);
                Ok(())
            }
            _ => Err(ValueError::TypeMismatch),
        }
    }

    /// If the field at index `n` is a radio field, toggles the radio button based on the value
    /// `choice`
    /// If it is not a radio button field or the choice is not a valid option, returns ValueError
    ///
    /// # Panics
    /// Will panic if n is larger than the number of fields
    pub fn set_radio(&mut self, n: usize, choice: String) -> Result<(), ValueError> {
        match self.get_state(n) {
            FieldState::Radio {
                selected: _,
                options,
            } => {
                if options.contains(&choice) {
                    let field = self
                        .doc
                        .objects
                        .get_mut(&self.form_ids[n])
                        .unwrap()
                        .as_dict_mut()
                        .unwrap();
                    field.set(b"V", Object::Name(choice.into_bytes()));
                    Ok(())
                } else {
                    Err(ValueError::InvalidSelection)
                }
            }
            _ => Err(ValueError::TypeMismatch),
        }
    }

    /// If the field at index `n` is a listbox field, selects the options in `choice`
    /// If it is not a listbox field or one of the choices is not a valid option, or if too many choices are selected, returns ValueError
    ///
    /// # Panics
    /// Will panic if n is larger than the number of fields
    pub fn set_list_box(&mut self, n: usize, choices: Vec<String>) -> Result<(), ValueError> {
        match self.get_state(n) {
            FieldState::ListBox {
                selected: _,
                options,
                multiselect,
            } => {
                if choices.iter().fold(true, |a, h| options.contains(h) && a) {
                    if !multiselect && choices.len() > 1 {
                        Err(ValueError::TooManySelected)
                    } else {
                        let field = self
                            .doc
                            .objects
                            .get_mut(&self.form_ids[n])
                            .unwrap()
                            .as_dict_mut()
                            .unwrap();
                        match choices.len() {
                            0 => field.set(b"V", Object::Null),
                            1 => field.set(
                                b"V",
                                Object::String(
                                    choices[0].clone().into_bytes(),
                                    StringFormat::Literal,
                                ),
                            ),
                            _ => field.set(
                                b"V",
                                Object::Array(
                                    choices
                                        .iter()
                                        .map(|x| {
                                            Object::String(
                                                x.clone().into_bytes(),
                                                StringFormat::Literal,
                                            )
                                        })
                                        .collect(),
                                ),
                            ),
                        };
                        Ok(())
                    }
                } else {
                    Err(ValueError::InvalidSelection)
                }
            }
            _ => Err(ValueError::TypeMismatch),
        }
    }

    /// If the field at index `n` is a combobox field, selects the options in `choice`
    /// If it is not a combobox field or one of the choices is not a valid option, or if too many choices are selected, returns ValueError
    ///
    /// # Panics
    /// Will panic if n is larger than the number of fields
    pub fn set_combo_box(&mut self, n: usize, choice: String) -> Result<(), ValueError> {
        match self.get_state(n) {
            FieldState::ComboBox {
                selected: _,
                options,
                editable,
            } => {
                if options.contains(&choice) || editable {
                    let field = self
                        .doc
                        .objects
                        .get_mut(&self.form_ids[n])
                        .unwrap()
                        .as_dict_mut()
                        .unwrap();
                    field.set(
                        b"V",
                        Object::String(choice.clone().into_bytes(), StringFormat::Literal),
                    );
                    Ok(())
                } else {
                    Err(ValueError::InvalidSelection)
                }
            }
            _ => Err(ValueError::TypeMismatch),
        }
    }

    /// Saves the form to the specified path
    pub fn save<P: AsRef<Path>>(&mut self, path: P) -> Result<(), io::Error> {
        self.doc.save(path).map(|_| ())
    }

    /// Saves the form to the specified path
    pub fn save_to<W: Write>(&mut self, target: &mut W) -> Result<(), io::Error> {
        self.doc.save_to(target)
    }
}
