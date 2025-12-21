macro_rules! define_page_size(
    ($Name:ident,$size:expr,$string:expr)=>{
        struct $Name;

        impl crate::mm::page_size::PageSize for $Name{
            const SIZE:usize=$size;
            const SIZE_STR:&str=$string;
        }
    }
);

pub trait PageSize {
	const SIZE: usize;
	const SIZE_STR: &str;
}
