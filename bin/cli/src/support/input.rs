use std::io::{self, Write};
use zeroize::Zeroizing;

/// Securely read a password from the user
pub fn read_password(prompt: &str) -> Result<Zeroizing<String>, Box<dyn std::error::Error>> {
    print!("{prompt}");
    io::stdout().flush()?;
    let password = rpassword::read_password()?;
    Ok(Zeroizing::new(password))
}

/// Read input from stdin or prompt user
pub fn read_input(prompt: &str, from_stdin: bool) -> Result<String, Box<dyn std::error::Error>> {
    if from_stdin {
        let mut input = String::new();
        io::stdin().read_line(&mut input)?;
        Ok(input.trim().to_string())
    } else {
        print!("{prompt}");
        io::stdout().flush()?;
        let mut input = String::new();
        io::stdin().read_line(&mut input)?;
        Ok(input.trim().to_string())
    }
}

/// Prompt user for confirmation
pub fn confirm(prompt: &str) -> Result<bool, Box<dyn std::error::Error>> {
    loop {
        print!("{prompt} (y/N): ");
        io::stdout().flush()?;
        let mut input = String::new();
        io::stdin().read_line(&mut input)?;

        match input.trim().to_lowercase().as_str() {
            "y" | "yes" => return Ok(true),
            "n" | "no" | "" => return Ok(false),
            _ => println!("Please enter 'y' or 'n'"),
        }
    }
}

pub fn print_warning(message: &str) {
    eprintln!("Warning: {message}");
}
